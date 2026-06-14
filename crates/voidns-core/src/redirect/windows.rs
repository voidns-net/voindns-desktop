//! Windows DNS redirect — ported from AmneziaVPN's `DnsUtilsWindows`.
//!
//! Source (MPL-2.0, Mozilla VPN lineage):
//!   client/platforms/windows/daemon/dnsutilswindows.cpp
//!   client/platforms/windows/daemon/dnsutilswindows.h
//!   <https://github.com/amnezia-vpn/amnezia-client/blob/dev/client/platforms/windows/daemon/dnsutilswindows.cpp>
//!
//! Original Amnezia code this module mirrors (verbatim, condensed):
//! ```cpp
//! bool DnsUtilsWindows::updateResolvers(const QString& ifname,
//!                                       const QList<QHostAddress>& resolvers) {
//!   MIB_IF_ROW2 entry;
//!   ConvertInterfaceAliasToLuid((wchar_t*)ifname.utf16(), &entry.InterfaceLuid);
//!   GetIfEntry2(&entry);
//!   m_luid = entry.InterfaceLuid.Value;
//!   if (m_setInterfaceDnsSettingsProcAddr == nullptr)
//!     return updateResolversNetsh(entry.InterfaceIndex, resolvers);
//!   return updateResolversWin32(entry.InterfaceGuid, resolvers);
//! }
//!
//! bool DnsUtilsWindows::updateResolversWin32(GUID guid, const QList<QHostAddress>& resolvers) {
//!   DNS_INTERFACE_SETTINGS settings;
//!   settings.Version = DNS_INTERFACE_SETTINGS_VERSION1;
//!   settings.Flags = DNS_SETTING_NAMESERVER | DNS_SETTING_SEARCHLIST;
//!   settings.Domain = nullptr; settings.NameServer = nullptr;
//!   settings.SearchList = (wchar_t*)L".";
//!   settings.RegistrationEnabled = false; settings.RegisterAdapterName = false;
//!   settings.EnableLLMNR = false; settings.QueryAdapterName = false;
//!   settings.ProfileNameServer = nullptr;
//!   settings.NameServer = (wchar_t*)v4resolverstring.utf16();      // IPv4
//!   DWORD v4result = m_setInterfaceDnsSettingsProcAddr(guid, &settings);
//!   settings.Flags |= DNS_SETTING_IPV6;                            // IPv6
//!   settings.NameServer = (wchar_t*)v6resolverstring.utf16();
//!   DWORD v6result = m_setInterfaceDnsSettingsProcAddr(guid, &settings);
//!   return (v4result == NO_ERROR) && (v6result == NO_ERROR);
//! }
//!
//! bool DnsUtilsWindows::restoreResolvers() {        // also called from ~DnsUtilsWindows
//!   if (m_luid == 0) return true;
//!   MIB_IF_ROW2 entry; entry.InterfaceLuid.Value = m_luid; GetIfEntry2(&entry);
//!   QList<QHostAddress> empty;                       // empty list clears the override
//!   return updateResolversWin32(entry.InterfaceGuid, empty);
//! }
//! ```
//!
//! Divergences from Amnezia (with reasons):
//! 1. Adapter selection — Amnezia configures the single VPN tunnel interface
//!    passed as `ifname`. voidns has no tunnel, so we enumerate every connected
//!    adapter via `netsh interface show interface` and point each at the proxy.
//! 2. LUID→GUID — Amnezia does ConvertInterfaceAliasToLuid → GetIfEntry2 →
//!    `InterfaceGuid`; we do ConvertInterfaceAliasToLuid → ConvertInterfaceLuidToGuid
//!    (same GUID, without materialising `MIB_IF_ROW2`).
//! 3. IPv4 only — Amnezia also sets IPv6 DNS (`DNS_SETTING_IPV6`). Our proxy binds
//!    `127.0.0.1` only, so pointing system IPv6 DNS at a non-listening address
//!    would break IPv6 resolution. IPv6 is a follow-up (proxy must also bind
//!    `[::1]:53`).
//! 4. API binding — Amnezia runtime-loads `SetInterfaceDnsSettings` via
//!    `GetProcAddress` to degrade to netsh on < Win10 20H1. We link it directly
//!    (windows-rs) and fall back to netsh on call error; the MVP targets
//!    Win10 20H1+ (a `GetProcAddress`/delay-load probe is a follow-up).
//! 5. Settings init — Amnezia sets each `DNS_INTERFACE_SETTINGS` field explicitly;
//!    we use `::default()` (zero-init → same values) then set the four fields.
//! 6. Restore — Amnezia re-resolves by saved LUID; we track the aliases we set and
//!    clear each (empty NameServer). Same effect: revert to DHCP-assigned DNS.

use std::iter::once;
use std::net::IpAddr;
use std::process::Command;

use anyhow::{anyhow, Result};
use tracing::{info, warn};
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToGuid, SetInterfaceDnsSettings,
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_NAMESERVER,
    DNS_SETTING_SEARCHLIST,
};
use windows::Win32::NetworkManagement::Ndis::NET_LUID_LH;

/// How DNS was applied, so restore takes the matching path.
enum Applied {
    None,
    Native(Vec<String>),
    Netsh(Vec<String>),
}

pub struct WindowsRedirector {
    applied: Applied,
}

impl WindowsRedirector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            applied: Applied::None,
        })
    }
}

impl super::DnsRedirector for WindowsRedirector {
    fn apply(&mut self, proxy: IpAddr) -> Result<()> {
        let ifaces = connected_interfaces()?;
        if ifaces.is_empty() {
            return Err(anyhow!("no connected network interfaces found"));
        }
        let server = proxy.to_string();

        // Native first.
        let mut native_ok = Vec::new();
        for name in &ifaces {
            match set_dns_native(name, &server) {
                Ok(()) => native_ok.push(name.clone()),
                Err(e) => warn!(iface = %name, error = %e, "native SetInterfaceDnsSettings failed"),
            }
        }
        if !native_ok.is_empty() {
            info!(
                count = native_ok.len(),
                "DNS redirected via SetInterfaceDnsSettings"
            );
            self.applied = Applied::Native(native_ok);
            return Ok(());
        }

        // Fallback: netsh.
        for name in &ifaces {
            netsh_set(name, &server)?;
        }
        info!(count = ifaces.len(), "DNS redirected via netsh (fallback)");
        self.applied = Applied::Netsh(ifaces);
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        match std::mem::replace(&mut self.applied, Applied::None) {
            Applied::Native(names) => {
                for name in names {
                    if let Err(e) = clear_dns_native(&name) {
                        warn!(iface = %name, error = %e, "native DNS restore failed");
                    }
                }
            }
            Applied::Netsh(names) => {
                for name in names {
                    let _ = netsh_clear(&name);
                }
            }
            Applied::None => {}
        }
        Ok(())
    }

    fn flush_cache(&self) -> Result<()> {
        run("ipconfig", &["/flushdns"]).map(|_| ())
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(once(0)).collect()
}

/// Resolve an interface alias (friendly name) to its adapter GUID.
fn alias_to_guid(alias: &str) -> Result<GUID> {
    let alias_w = wide(alias);
    unsafe {
        let mut luid = NET_LUID_LH::default();
        let rc = ConvertInterfaceAliasToLuid(PCWSTR(alias_w.as_ptr()), &mut luid);
        if rc != ERROR_SUCCESS {
            return Err(anyhow!("ConvertInterfaceAliasToLuid({alias}) -> {}", rc.0));
        }
        let mut guid = GUID::zeroed();
        let rc = ConvertInterfaceLuidToGuid(&luid, &mut guid);
        if rc != ERROR_SUCCESS {
            return Err(anyhow!("ConvertInterfaceLuidToGuid -> {}", rc.0));
        }
        Ok(guid)
    }
}

fn set_dns_native(alias: &str, server: &str) -> Result<()> {
    let guid = alias_to_guid(alias)?;
    let mut ns = wide(server);
    let mut search = wide(".");
    let mut settings = DNS_INTERFACE_SETTINGS::default();
    settings.Version = DNS_INTERFACE_SETTINGS_VERSION1;
    settings.Flags = (DNS_SETTING_NAMESERVER | DNS_SETTING_SEARCHLIST) as u64;
    settings.NameServer = PWSTR(ns.as_mut_ptr());
    settings.SearchList = PWSTR(search.as_mut_ptr());
    unsafe {
        let rc = SetInterfaceDnsSettings(guid, &settings);
        if rc != ERROR_SUCCESS {
            return Err(anyhow!("SetInterfaceDnsSettings -> {}", rc.0));
        }
    }
    Ok(())
}

/// Clear the override (empty nameserver) → reverts the adapter to DHCP DNS.
fn clear_dns_native(alias: &str) -> Result<()> {
    let guid = alias_to_guid(alias)?;
    let mut empty = wide("");
    let mut settings = DNS_INTERFACE_SETTINGS::default();
    settings.Version = DNS_INTERFACE_SETTINGS_VERSION1;
    settings.Flags = (DNS_SETTING_NAMESERVER | DNS_SETTING_SEARCHLIST) as u64;
    settings.NameServer = PWSTR(empty.as_mut_ptr());
    settings.SearchList = PWSTR(empty.as_mut_ptr());
    unsafe {
        let rc = SetInterfaceDnsSettings(guid, &settings);
        if rc != ERROR_SUCCESS {
            return Err(anyhow!("SetInterfaceDnsSettings(clear) -> {}", rc.0));
        }
    }
    Ok(())
}

fn netsh_set(name: &str, server: &str) -> Result<()> {
    run(
        "netsh",
        &[
            "interface",
            "ipv4",
            "set",
            "dnsservers",
            &format!("name={name}"),
            "static",
            server,
            "primary",
        ],
    )
    .map(|_| ())
}

fn netsh_clear(name: &str) -> Result<()> {
    run(
        "netsh",
        &[
            "interface",
            "ipv4",
            "set",
            "dnsservers",
            &format!("name={name}"),
            "dhcp",
        ],
    )
    .map(|_| ())
}

/// Connected interface aliases from `netsh interface show interface`.
fn connected_interfaces() -> Result<Vec<String>> {
    let out = Command::new("netsh")
        .args(["interface", "show", "interface"])
        .output()?;
    Ok(super::parse::parse_netsh_interfaces(
        &String::from_utf8_lossy(&out.stdout),
    ))
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
