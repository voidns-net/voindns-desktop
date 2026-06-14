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

/// Construct the redirector for the current OS.
pub fn new_redirector() -> Result<Box<dyn DnsRedirector>> {
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
