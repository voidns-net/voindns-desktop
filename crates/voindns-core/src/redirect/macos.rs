//! macOS DNS redirect.
//!
//! Primary: native `SCDynamicStore` writing `State:/Network/Service/<uuid>/DNS`
//! — mirrors AmneziaVPN's `DnsUtilsMacos`. Writing to the runtime `State:/` tree
//! (not persisted `Setup:/`) means a crash auto-reverts. On restore we remove
//! our override so the service falls back to its `Setup:/` (user) DNS. Falls
//! back to `networksetup` if the SCDynamicStore session can't be created.
//!
//! `SCDynamicStore`/`CFPropertyList` aren't `Send`, so they're built per call
//! and only `String` paths are retained on the (Send) redirector.

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
    let store = SCDynamicStoreBuilder::new("voindns")
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
    let store = SCDynamicStoreBuilder::new("voindns")
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
