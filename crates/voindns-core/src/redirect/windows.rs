//! Windows DNS redirect.
//!
//! Primary: native `SetInterfaceDnsSettings` (IP Helper) — mirrors AmneziaVPN's
//! `DnsUtilsWindows`. Falls back to `netsh` (Amnezia's documented fallback) when
//! the native call is unavailable (< Win10 20H1) or errors. Active adapters are
//! enumerated via `netsh interface show interface` (read-only; parser-tested),
//! then each alias's LUID→GUID is resolved and DNS set natively. IPv4 only for
//! the MVP (matches prior behaviour; IPv6 is a tracked follow-up).

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
