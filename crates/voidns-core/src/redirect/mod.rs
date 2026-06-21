//! System DNS redirection. One trait, three OS implementations. We point the
//! system resolver at the local proxy (`127.0.0.1`) on Connect and restore the
//! exact prior state on Disconnect — mirroring AmneziaVPN's `DnsUtils` contract
//! (`updateResolvers` / `restoreResolvers`).
//!
//! Methods are synchronous and may block (D-Bus / subprocess); the controller
//! invokes them via `tokio::task::block_in_place`.

use std::net::IpAddr;

use anyhow::Result;

pub trait DnsRedirector: Send {
    /// Snapshot current DNS and route all queries to `proxy`.
    fn apply(&mut self, proxy: IpAddr) -> Result<()>;
    /// Restore the pre-connect DNS state. Idempotent.
    fn restore(&mut self) -> Result<()>;
    /// Flush the OS DNS cache.
    fn flush_cache(&self) -> Result<()>;
}

mod parse;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

/// A redirector that does nothing. Selected when `VOIDNS_SKIP_REDIRECT` is set,
/// so an unprivileged run (local dev, the e2e harness in `--mode dev`) can start
/// the proxy and be queried directly on its loopback port without touching the
/// system resolver. Never engaged in production or the privileged installer e2e,
/// where the variable is unset and the real OS redirector runs.
struct NoopRedirector;

impl DnsRedirector for NoopRedirector {
    fn apply(&mut self, _proxy: IpAddr) -> Result<()> {
        Ok(())
    }
    fn restore(&mut self) -> Result<()> {
        Ok(())
    }
    fn flush_cache(&self) -> Result<()> {
        Ok(())
    }
}

/// Construct the redirector for the current OS.
pub fn new_redirector() -> Result<Box<dyn DnsRedirector>> {
    if std::env::var_os("VOIDNS_SKIP_REDIRECT").is_some() {
        tracing::warn!("VOIDNS_SKIP_REDIRECT set: system DNS will NOT be redirected");
        return Ok(Box::new(NoopRedirector));
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::LinuxRedirector::new()?))
    }
    #[cfg(target_os = "windows")]
    {
        Ok(Box::new(windows::WindowsRedirector::new()?))
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::MacosRedirector::new()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        anyhow::bail!("DNS redirection is not implemented for this platform")
    }
}
