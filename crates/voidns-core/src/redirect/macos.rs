//! macOS DNS redirect — ported from AmneziaVPN's `DnsUtilsMacos`.
//!
//! Source (MPL-2.0, Mozilla VPN lineage):
//!   client/platforms/macos/daemon/dnsutilsmacos.cpp
//!   client/platforms/macos/daemon/dnsutilsmacos.h
//!   <https://github.com/amnezia-vpn/amnezia-client/blob/dev/client/platforms/macos/daemon/dnsutilsmacos.cpp>
//!
//! Original Amnezia code this module mirrors (verbatim, condensed):
//! ```cpp
//! m_scStore = SCDynamicStoreCreate(kCFAllocatorSystemDefault,
//!                                  CFSTR("amneziavpn"), nullptr, nullptr);
//!
//! bool DnsUtilsMacos::updateResolvers(const QString& ifname,
//!                                     const QList<QHostAddress>& resolvers) {
//!   CFArrayRef netServices = SCDynamicStoreCopyKeyList(
//!       m_scStore, CFSTR("Setup:/Network/Service/[0-9A-F-]+"));
//!   CFMutableDictionaryRef dnsConfig = CFDictionaryCreateMutable(...);
//!   cfDictSetStringList(dnsConfig, kSCPropNetDNSServerAddresses, list);
//!   cfDictSetString(dnsConfig, kSCPropNetDNSDomainName, "lan");
//!   for (CFIndex i = 0; i < CFArrayGetCount(netServices); i++) {
//!     QString uuid = service.section('/', 3, 3);
//!     backupService(uuid);                              // snapshot prior DNS dict
//!     CFStringRef dnsPath = CFStringCreateWithFormat(...,
//!         CFSTR("Setup:/Network/Service/%s/DNS"), qPrintable(uuid));
//!     SCDynamicStoreSetValue(m_scStore, dnsPath, dnsConfig);   // <-- writes Setup:/
//!   }
//!   return true;
//! }
//!
//! void DnsUtilsMacos::backupService(const QString& uuid) {     // reads + stores
//!   CFDictionaryRef config = SCDynamicStoreCopyValue(m_scStore,
//!       "Setup:/Network/Service/<uuid>/DNS");                  // domain/search/servers/sortlist
//! }
//!
//! bool DnsUtilsMacos::restoreResolvers() {            // also from ~DnsUtilsMacos
//!   for (uuid : m_prevServices.keys()) {
//!     if (backup.isValid()) SCDynamicStoreSetValue(m_scStore, path, savedConfig);
//!     else                  SCDynamicStoreRemoveValue(m_scStore, path);
//!   }
//! }
//! ```
//!
//! Divergences from Amnezia (with reasons):
//! 1. `State:/` instead of `Setup:/` (DELIBERATE) — Amnezia writes the override
//!    into the persisted `Setup:/Network/Service/<uuid>/DNS` tree and must
//!    therefore snapshot the prior dict (domain/search/servers/sortlist) and
//!    rewrite it on restore, or it would destroy a user's static DNS. We write
//!    the runtime `State:/…/DNS` tree (the Mullvad approach): it takes precedence
//!    for resolution, auto-reverts on crash/exit (ephemeral), and on restore we
//!    simply `remove` our key — the service falls back to its untouched `Setup:/`
//!    (user) DNS with no snapshot needed. This avoids `CFPropertyList`-read
//!    backup code that can only be compiled on a macOS runner (CI currently
//!    blocked by the org account), and is strictly safer for the user's config.
//!    A faithful `Setup:/`+backup port is a follow-up once macOS CI is unblocked.
//! 2. `kSCPropNetDNSDomainName = "lan"` omitted — Amnezia sets it; its purpose is
//!    unclear and unnecessary for a catch-all local proxy, so we set only
//!    `ServerAddresses`.
//! 3. `networksetup` fallback added — Amnezia's `DnsUtilsMacos` is pure
//!    SCDynamicStore. We fall back to `networksetup -setdnsservers` if the
//!    SCDynamicStore session can't be created (e.g. sandboxing).
//! 4. Per-call store — Amnezia holds one `SCDynamicStoreRef`; `SCDynamicStore` /
//!    `CFPropertyList` aren't `Send`, so we build the store inside each call and
//!    retain only `String` paths on the (Send) redirector.

use std::net::IpAddr;
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use core_foundation::array::CFArray;
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::propertylist::CFPropertyListSubClass;
use core_foundation::string::CFString;
use system_configuration::dynamic_store::SCDynamicStoreBuilder;
use tracing::{info, warn};

const SERVICE_PATTERN: &str = "Setup:/Network/Service/[0-9A-F-]+";

/// How DNS was applied, so restore takes the matching path.
enum Applied {
    None,
    /// `State:/…/DNS` paths we set.
    Native(Vec<String>),
    /// service name -> previous DNS servers (empty = was DHCP/none).
    Shell(Vec<(String, Vec<String>)>),
}

pub struct MacosRedirector {
    applied: Applied,
}

impl MacosRedirector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            applied: Applied::None,
        })
    }
}

impl super::DnsRedirector for MacosRedirector {
    fn apply(&mut self, proxy: IpAddr) -> Result<()> {
        match apply_native(proxy) {
            Ok(paths) => {
                info!(count = paths.len(), "DNS redirected via SCDynamicStore");
                self.applied = Applied::Native(paths);
                let _ = self.flush_cache();
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "SCDynamicStore path failed; falling back to networksetup");
                let saved = apply_shell(proxy)?;
                info!(
                    count = saved.len(),
                    "DNS redirected via networksetup (fallback)"
                );
                self.applied = Applied::Shell(saved);
                let _ = self.flush_cache();
                Ok(())
            }
        }
    }

    fn restore(&mut self) -> Result<()> {
        match std::mem::replace(&mut self.applied, Applied::None) {
            Applied::Native(paths) => restore_native(&paths)?,
            Applied::Shell(prev) => restore_shell(prev),
            Applied::None => {}
        }
        Ok(())
    }

    fn flush_cache(&self) -> Result<()> {
        let _ = run("dscacheutil", &["-flushcache"]);
        let _ = run("killall", &["-HUP", "mDNSResponder"]);
        Ok(())
    }
}

// --- native SCDynamicStore ---

fn apply_native(proxy: IpAddr) -> Result<Vec<String>> {
    let store = SCDynamicStoreBuilder::new("voidns")
        .build()
        .ok_or_else(|| anyhow!("SCDynamicStoreCreate failed"))?;

    let keys = store
        .get_keys(SERVICE_PATTERN)
        .ok_or_else(|| anyhow!("no network services found"))?;

    // DNS config dict: { "ServerAddresses": ["<proxy>"] }
    let server = CFString::new(&proxy.to_string());
    let addrs = CFArray::from_CFTypes(&[server]);
    let key = CFString::from_static_string("ServerAddresses");
    let dict = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), addrs.as_CFType())]);
    let plist = dict.to_untyped().to_CFPropertyList();

    let mut applied = Vec::new();
    for k in keys.iter() {
        let setup = k.to_string(); // "Setup:/Network/Service/<uuid>"
        let uuid = setup.rsplit('/').next().unwrap_or_default();
        let state_path = format!("State:/Network/Service/{uuid}/DNS");
        if store.set_raw(state_path.as_str(), &plist) {
            applied.push(state_path);
        }
    }
    if applied.is_empty() {
        bail!("failed to set DNS on any service");
    }
    Ok(applied)
}

fn restore_native(paths: &[String]) -> Result<()> {
    let store = SCDynamicStoreBuilder::new("voidns")
        .build()
        .ok_or_else(|| anyhow!("SCDynamicStoreCreate failed"))?;
    for path in paths {
        store.remove(path.as_str());
    }
    Ok(())
}

// --- networksetup fallback ---

fn apply_shell(proxy: IpAddr) -> Result<Vec<(String, Vec<String>)>> {
    let services = network_services()?;
    if services.is_empty() {
        bail!("no network services found");
    }
    let mut saved = Vec::new();
    for svc in &services {
        let prev = current_dns(svc).unwrap_or_default();
        run("networksetup", &["-setdnsservers", svc, &proxy.to_string()])?;
        saved.push((svc.clone(), prev));
    }
    Ok(saved)
}

fn restore_shell(prev: Vec<(String, Vec<String>)>) {
    for (svc, servers) in prev {
        let mut args: Vec<&str> = vec!["-setdnsservers", svc.as_str()];
        if servers.is_empty() {
            args.push("Empty");
        } else {
            args.extend(servers.iter().map(|s| s.as_str()));
        }
        if let Err(e) = run("networksetup", &args) {
            warn!(service = %svc, error = %e, "failed to restore DNS");
        }
    }
}

fn network_services() -> Result<Vec<String>> {
    let out = run("networksetup", &["-listallnetworkservices"])?;
    Ok(super::parse::parse_network_services(&out))
}

fn current_dns(service: &str) -> Option<Vec<String>> {
    let out = run("networksetup", &["-getdnsservers", service]).ok()?;
    Some(super::parse::parse_dns_servers(&out))
}

fn run(cmd: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(cmd).args(args).output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "{cmd} {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
