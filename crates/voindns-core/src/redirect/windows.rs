//! Windows DNS redirect (MVP via `netsh`).
//!
//! AmneziaVPN's primary path is the native `SetInterfaceDnsSettings` IP Helper
//! API, with `netsh` as its documented fallback. The MVP uses `netsh` (robust,
//! no FFI); the native windows-rs implementation is a tracked follow-up
//! (plan §6.1). Sets the static DNS of every connected IPv4 interface to the
//! proxy and reverts to DHCP on restore.

use std::net::IpAddr;
use std::process::Command;

use anyhow::{anyhow, Result};
use tracing::{info, warn};

pub struct WindowsRedirector {
    /// Interface aliases we overrode, for restore.
    applied: Vec<String>,
}

impl WindowsRedirector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            applied: Vec::new(),
        })
    }
}

impl super::DnsRedirector for WindowsRedirector {
    fn apply(&mut self, proxy: IpAddr) -> Result<()> {
        let ifaces = connected_interfaces()?;
        if ifaces.is_empty() {
            return Err(anyhow!("no connected network interfaces found"));
        }
        for name in &ifaces {
            run(
                "netsh",
                &[
                    "interface",
                    "ipv4",
                    "set",
                    "dnsservers",
                    &format!("name={name}"),
                    "static",
                    &proxy.to_string(),
                    "primary",
                ],
            )?;
        }
        info!(count = ifaces.len(), "DNS redirected via netsh");
        self.applied = ifaces;
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        for name in std::mem::take(&mut self.applied) {
            if let Err(e) = run(
                "netsh",
                &[
                    "interface",
                    "ipv4",
                    "set",
                    "dnsservers",
                    &format!("name={name}"),
                    "dhcp",
                ],
            ) {
                warn!(iface = %name, error = %e, "failed to restore DNS");
            }
        }
        Ok(())
    }

    fn flush_cache(&self) -> Result<()> {
        run("ipconfig", &["/flushdns"]).map(|_| ())
    }
}

/// Connected interface aliases from `netsh interface show interface`.
fn connected_interfaces() -> Result<Vec<String>> {
    let out = Command::new("netsh")
        .args(["interface", "show", "interface"])
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(super::parse::parse_netsh_interfaces(&text))
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
