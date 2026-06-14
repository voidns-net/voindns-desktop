//! Linux DNS redirect — ported from AmneziaVPN's `DnsUtilsLinux`.
//!
//! Source (MPL-2.0, Mozilla VPN lineage):
//!   client/platforms/linux/daemon/dnsutilslinux.cpp
//!   client/platforms/linux/daemon/dnsutilslinux.h
//!   <https://github.com/amnezia-vpn/amnezia-client/blob/dev/client/platforms/linux/daemon/dnsutilslinux.cpp>
//!
//! Original Amnezia code this module mirrors (verbatim, condensed):
//! ```cpp
//! // Constructor: QDBusInterface to org.freedesktop.resolve1 on the system bus.
//! m_resolver = new QDBusInterface("org.freedesktop.resolve1",
//!     "/org/freedesktop/resolve1", "org.freedesktop.resolve1.Manager",
//!     QDBusConnection::systemBus(), this);
//!
//! bool DnsUtilsLinux::updateResolvers(const QString& ifname,
//!                                     const QList<QHostAddress>& resolvers) {
//!   m_ifindex = if_nametoindex(qPrintable(ifname));
//!   setLinkDNS(m_ifindex, resolvers);
//!   setLinkDefaultRoute(m_ifindex, true);
//!   updateLinkDomains();
//!   return true;
//! }
//!
//! void DnsUtilsLinux::setLinkDNS(int ifindex, const QList<QHostAddress>& resolvers) {
//!   QList<DnsResolver> resolverList; for (auto& ip : resolvers) resolverList.append(ip);
//!   argumentList << QVariant::fromValue(ifindex) << QVariant::fromValue(resolverList);
//!   m_resolver->asyncCallWithArgumentList("SetLinkDNS", argumentList);
//! }
//! void DnsUtilsLinux::setLinkDefaultRoute(int ifindex, bool enable) {
//!   argumentList << QVariant::fromValue(ifindex) << QVariant::fromValue(enable);
//!   m_resolver->asyncCallWithArgumentList("SetLinkDefaultRoute", argumentList);
//! }
//!
//! bool DnsUtilsLinux::restoreResolvers() {          // also from ~DnsUtilsLinux
//!   for (auto it = m_linkDomains.constBegin(); it != m_linkDomains.constEnd(); ++it)
//!     setLinkDomains(it.key(), it.value());          // restore other links' domains
//!   m_linkDomains.clear();
//!   if (m_ifindex > 0) {
//!     m_resolver->asyncCallWithArgumentList("RevertLink", {QVariant::fromValue(m_ifindex)});
//!     m_ifindex = 0;
//!   }
//!   return true;
//! }
//! ```
//!
//! Divergences from Amnezia (with reasons):
//! 1. Interface selection — Amnezia uses the VPN tunnel name passed as `ifname`
//!    (`if_nametoindex`). voidns has no tunnel, so we resolve the default-route
//!    interface from `/proc/net/route` and target that.
//! 2. `SetLinkDomains`/`updateLinkDomains` omitted — Amnezia also snapshots and
//!    rewrites other links' search/routing domains (setting `~.` on its link and
//!    removing it from competitors). We rely on `SetLinkDefaultRoute(ifindex,true)`
//!    alone, which makes our link the default DNS route — sufficient for the
//!    common single-link case. Multi-link domain arbitration is a follow-up.
//! 3. `/etc/resolv.conf` fallback added — Amnezia's `DnsUtilsLinux` is pure D-Bus
//!    (assumes systemd-resolved). We add a resolv.conf backup/rewrite fallback so
//!    non-resolved systems still work.
//! 4. Sync vs async — Amnezia uses `asyncCallWithArgumentList` (Qt async D-Bus);
//!    we use the `zbus` blocking `Proxy::call` (driven via `block_in_place`).
//!    Same SetLinkDNS / SetLinkDefaultRoute / RevertLink calls.

use std::fs;
use std::net::IpAddr;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use tracing::{info, warn};

const RESOLVED_STUB: &str = "/run/systemd/resolve/stub-resolv.conf";
const RESOLV_CONF: &str = "/etc/resolv.conf";
const RESOLV_BACKUP: &str = "/etc/resolv.conf.voidns.bak";

const RESOLVE1_SERVICE: &str = "org.freedesktop.resolve1";
const RESOLVE1_PATH: &str = "/org/freedesktop/resolve1";
const RESOLVE1_IFACE: &str = "org.freedesktop.resolve1.Manager";

enum Active {
    Idle,
    Resolved { ifindex: i32 },
    ResolvConf,
}

pub struct LinuxRedirector {
    active: Active,
}

impl LinuxRedirector {
    pub fn new() -> Result<Self> {
        Ok(Self {
            active: Active::Idle,
        })
    }
}

impl super::DnsRedirector for LinuxRedirector {
    fn apply(&mut self, proxy: IpAddr) -> Result<()> {
        if resolved_active() {
            let iface =
                default_route_iface().context("could not determine default-route interface")?;
            let ifindex = nix::net::if_::if_nametoindex(iface.as_str())
                .with_context(|| format!("if_nametoindex({iface})"))?
                as i32;
            set_link_dns(ifindex, proxy)?;
            set_link_default_route(ifindex, true)?;
            info!(%iface, ifindex, "DNS redirected via systemd-resolved");
            self.active = Active::Resolved { ifindex };
        } else {
            resolvconf_apply(proxy)?;
            info!("DNS redirected via /etc/resolv.conf rewrite");
            self.active = Active::ResolvConf;
        }
        Ok(())
    }

    fn restore(&mut self) -> Result<()> {
        match std::mem::replace(&mut self.active, Active::Idle) {
            Active::Resolved { ifindex } => {
                revert_link(ifindex).context("RevertLink")?;
                info!(ifindex, "DNS restored (RevertLink)");
            }
            Active::ResolvConf => {
                resolvconf_restore()?;
                info!("DNS restored (/etc/resolv.conf)");
            }
            Active::Idle => {}
        }
        Ok(())
    }

    fn flush_cache(&self) -> Result<()> {
        if resolved_active() {
            if let Err(e) = manager_call::<()>("FlushCaches", &()) {
                warn!(error = %e, "FlushCaches failed");
            }
        }
        Ok(())
    }
}

fn resolved_active() -> bool {
    Path::new(RESOLVED_STUB).exists()
}

/// Interface owning the default route (parsed from `/proc/net/route`).
fn default_route_iface() -> Option<String> {
    let table = fs::read_to_string("/proc/net/route").ok()?;
    super::parse::parse_default_route_iface(&table)
}

// --- systemd-resolved D-Bus (blocking) ---

fn manager_call<R>(
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    let conn = zbus::blocking::Connection::system().context("connect system bus")?;
    let proxy = zbus::blocking::Proxy::new(&conn, RESOLVE1_SERVICE, RESOLVE1_PATH, RESOLVE1_IFACE)
        .context("resolve1 proxy")?;
    proxy
        .call::<_, _, R>(method, body)
        .map_err(|e| anyhow!("{method}: {e}"))
}

fn set_link_dns(ifindex: i32, proxy: IpAddr) -> Result<()> {
    let (family, bytes): (i32, Vec<u8>) = match proxy {
        IpAddr::V4(a) => (libc_af_inet(), a.octets().to_vec()),
        IpAddr::V6(a) => (libc_af_inet6(), a.octets().to_vec()),
    };
    // SetLinkDNS(i ifindex, a(iay) addresses)
    let addrs: Vec<(i32, Vec<u8>)> = vec![(family, bytes)];
    manager_call::<()>("SetLinkDNS", &(ifindex, addrs))
}

fn set_link_default_route(ifindex: i32, enable: bool) -> Result<()> {
    // SetLinkDefaultRoute(i ifindex, b enable)
    manager_call::<()>("SetLinkDefaultRoute", &(ifindex, enable))
}

fn revert_link(ifindex: i32) -> Result<()> {
    // RevertLink(i ifindex)
    manager_call::<()>("RevertLink", &(ifindex,))
}

fn libc_af_inet() -> i32 {
    2
}
fn libc_af_inet6() -> i32 {
    10
}

// --- /etc/resolv.conf fallback ---

fn resolvconf_apply(proxy: IpAddr) -> Result<()> {
    if Path::new(RESOLV_CONF).exists() && !Path::new(RESOLV_BACKUP).exists() {
        // NOTE: if resolv.conf is a symlink this copies the target's contents,
        // which is acceptable for restore. The native NetworkManager/resolvconf
        // integration is a tracked follow-up.
        fs::copy(RESOLV_CONF, RESOLV_BACKUP).context("backup resolv.conf")?;
    }
    if let Ok(meta) = fs::symlink_metadata(RESOLV_CONF) {
        if meta.file_type().is_symlink() {
            fs::remove_file(RESOLV_CONF).context("unlink resolv.conf symlink")?;
        }
    }
    fs::write(
        RESOLV_CONF,
        format!("# voidns managed\nnameserver {proxy}\n"),
    )
    .context("write resolv.conf")
}

fn resolvconf_restore() -> Result<()> {
    if Path::new(RESOLV_BACKUP).exists() {
        fs::copy(RESOLV_BACKUP, RESOLV_CONF).context("restore resolv.conf")?;
        let _ = fs::remove_file(RESOLV_BACKUP);
    }
    Ok(())
}
