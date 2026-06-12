//! macOS DNS redirect (MVP via `networksetup`).
//!
//! AmneziaVPN uses the native SCDynamicStore (`State:/…/DNS`) API. The MVP uses
//! `networksetup` (supported CLI, no FFI): set every network service's DNS to
//! the proxy and restore the captured prior value (or "Empty" → DHCP). The
//! native SCDynamicStore implementation is a tracked follow-up (plan §6.3).

use std::net::IpAddr;
use std::process::Command;

use anyhow::{anyhow, Result};
use tracing::{info, warn};

pub struct MacosRedirector {
    /// service name -> previous DNS servers (empty = was DHCP/none).
    previous: Vec<(String, Vec<String>)>,
}

impl MacosRedirector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            previous: Vec::new(),
        })
    }
}

impl super::DnsRedirector for MacosRedirector {
    fn apply(&mut self, proxy: IpAddr) -> Result<()> {
        let services = network_services()?;
        if services.is_empty() {
            return Err(anyhow!("no network services found"));
        }
        let mut saved = Vec::new();
        for svc in &services {
            let prev = current_dns(svc).unwrap_or_default();
            run("networksetup", &["-setdnsservers", svc, &proxy.to_string()])?;
            saved.push((svc.clone(), prev));
        }
        info!(count = services.len(), "DNS redirected via networksetup");
        self.previous = saved;
        let _ = self.flush_cache();
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        for (svc, prev) in std::mem::take(&mut self.previous) {
            let mut args: Vec<&str> = vec!["-setdnsservers", svc.as_str()];
            if prev.is_empty() {
                args.push("Empty");
            } else {
                args.extend(prev.iter().map(|s| s.as_str()));
            }
            if let Err(e) = run("networksetup", &args) {
                warn!(service = %svc, error = %e, "failed to restore DNS");
            }
        }
        Ok(())
    }

    fn flush_cache(&self) -> Result<()> {
        let _ = run("dscacheutil", &["-flushcache"]);
        let _ = run("killall", &["-HUP", "mDNSResponder"]);
        Ok(())
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
