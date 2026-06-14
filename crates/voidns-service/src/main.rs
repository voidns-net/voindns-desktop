//! voidns-service: the privileged background daemon. Runs the DoH proxy +
//! DNS redirector and serves the GUI over local IPC.
//!
//! This is the elevated half of the split-privilege design (mirrors AmneziaVPN):
//! installed once as a root systemd unit, it does the privileged DNS work so the
//! unprivileged GUI never needs root — it just sends commands over the local
//! socket. See installers/linux/voidns.service + install-dev.sh.
//!
//! Modes (argv[1]): `run` (default, foreground), `install`, `uninstall`.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use voidns_core::Controller;
use voidns_proto::DEFAULT_PROXY_PORT;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    match std::env::args().nth(1).as_deref() {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        _ => run().await,
    }
}

async fn run() -> Result<()> {
    let port = std::env::var("VOIDNS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PROXY_PORT);

    let controller = Arc::new(Mutex::new(Controller::new(port)?));
    tracing::info!(port, "voidns-service starting");

    // systemd Type=notify readiness (no-op if not under systemd).
    #[cfg(target_os = "linux")]
    {
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);
    }

    let server = voidns_core::ipc::serve(controller.clone());
    tokio::select! {
        res = server => res?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown signal received; restoring DNS");
            controller.lock().await.disconnect().await;
        }
    }
    Ok(())
}

fn install() -> Result<()> {
    println!(
        "Service installation is performed by the platform installer:\n\
         - Linux:   installers/linux/install-dev.sh  (or the .deb/.rpm postinst)\n\
         - Windows: installers/windows  (sc create / NSIS hook)\n\
         - macOS:   installers/macos    (LaunchDaemon + pkg postinstall)\n"
    );
    Ok(())
}

fn uninstall() -> Result<()> {
    println!("Use the platform package manager / installer to remove the voidns service.");
    Ok(())
}
