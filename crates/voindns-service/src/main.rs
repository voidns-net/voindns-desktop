//! voindns-service: the privileged background daemon. Runs the DoH proxy +
//! DNS redirector and serves the GUI over local IPC.
//!
//! Modes (argv[1]): `run` (default, foreground), `install`, `uninstall`.
//! Native service-manager integration (Windows SCM / systemd / launchd) is
//! wired here; this MVP runs in the foreground and is launched by the per-OS
//! installer units under `installers/`.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use voindns_core::Controller;
use voindns_proto::DEFAULT_PROXY_PORT;

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
    let port = std::env::var("VOINDNS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PROXY_PORT);

    let controller = Arc::new(Mutex::new(Controller::new(port)?));
    tracing::info!(port, "voindns-service starting");

    // systemd Type=notify readiness (no-op if not under systemd).
    #[cfg(target_os = "linux")]
    {
        let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);
    }

    let server = voindns_core::ipc::serve(controller.clone());
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
         - Linux:   installers/linux/voindns.service  (systemctl enable --now voindns)\n\
         - Windows: installers/windows  (sc create / NSIS hook)\n\
         - macOS:   installers/macos    (LaunchDaemon + pkg postinstall)\n"
    );
    Ok(())
}

fn uninstall() -> Result<()> {
    println!("Use the platform package manager / installer to remove the voindns service.");
    Ok(())
}
