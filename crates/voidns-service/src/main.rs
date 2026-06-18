//! voidns-service: the privileged background daemon. Runs the DoH proxy +
//! DNS redirector and serves the GUI over local IPC.
//!
//! This is the elevated half of the split-privilege design (mirrors AmneziaVPN):
//! installed once as a root systemd unit, it does the privileged DNS work so the
//! unprivileged GUI never needs root — it just sends commands over the local
//! socket. See installers/linux/voidns.service + install-dev.sh.
//!
//! Modes (argv[1]):
//!   * `run` (default, foreground) — the daemon: proxy + redirector + IPC server.
//!   * `install` / `uninstall` — installer hints.
//!   * `connect <upstream>` / `disconnect` / `status` — control an already-running
//!     daemon over IPC (the same commands the GUI sends). Lets the service be
//!     driven from a terminal / script. `<upstream>` is one of
//!     `cloudflare|google|quad9|voidns`, or `custom <ip> <hostname> <path> <port>`.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;
use voidns_core::{ipc, Controller};
use voidns_proto::{Command, ConnState, Event, UpstreamSel, DEFAULT_PROXY_PORT};

#[cfg(windows)]
mod winservice;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("install") => install(),
        Some("uninstall") => uninstall(),
        Some("connect") => block_on(ctl(Command::Connect {
            upstream: parse_upstream(&args[2..])?,
        })),
        Some("disconnect") => block_on(ctl(Command::Disconnect)),
        Some("status") => block_on(ctl(Command::GetStatus)),
        _ => run(),
    }
}

/// Build a multi-thread Tokio runtime and drive `fut` to completion. Used for
/// the foreground daemon and the one-shot control commands. (Not a
/// `#[tokio::main]` because the Windows service path must own the runtime — it
/// is created inside the SCM dispatcher thread, not around `main`.)
fn block_on<F: std::future::Future<Output = Result<()>>>(fut: F) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?
        .block_on(fut)
}

/// Parse a CLI upstream spec into an [`UpstreamSel`].
fn parse_upstream(args: &[String]) -> Result<UpstreamSel> {
    match args.first().map(String::as_str) {
        Some("cloudflare") => Ok(UpstreamSel::Cloudflare),
        Some("google") => Ok(UpstreamSel::Google),
        Some("quad9") => Ok(UpstreamSel::Quad9),
        Some("voidns") => Ok(UpstreamSel::Voidns),
        Some("custom") => Ok(UpstreamSel::Custom {
            ip: args.get(1).context("custom: missing <ip>")?.clone(),
            hostname: args.get(2).context("custom: missing <hostname>")?.clone(),
            path: args.get(3).context("custom: missing <path>")?.clone(),
            port: args
                .get(4)
                .context("custom: missing <port>")?
                .parse()
                .context("custom: invalid <port>")?,
        }),
        _ => bail!(
            "usage: voidns-service connect <cloudflare|google|quad9|voidns|custom <ip> <hostname> <path> <port>>"
        ),
    }
}

/// Send one control command to the running daemon over IPC and report the reply.
/// Exits non-zero if the daemon reports an error (or a non-connected state for a
/// connect) so scripts can gate on it.
async fn ctl(cmd: Command) -> Result<()> {
    let connecting = matches!(cmd, Command::Connect { .. });
    match ipc::one_shot(&cmd).await? {
        Event::Status(s) => {
            println!(
                "state={:?} upstream={} listen={} error={}",
                s.state,
                s.upstream.as_deref().unwrap_or("-"),
                s.listen.as_deref().unwrap_or("-"),
                s.error.as_deref().unwrap_or("-"),
            );
            if matches!(s.state, ConnState::Error)
                || (connecting && s.state != ConnState::Connected)
            {
                std::process::exit(1);
            }
        }
        Event::Pong => println!("pong"),
        Event::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// The `run` mode (daemon). On Windows, first try to attach to the Service
/// Control Manager; if we were not launched by the SCM, fall through to
/// foreground. Everywhere else: foreground until Ctrl-C / SIGTERM.
fn run() -> Result<()> {
    #[cfg(windows)]
    {
        if winservice::try_run_as_service()? {
            return Ok(());
        }
    }
    block_on(run_daemon(async {
        let _ = tokio::signal::ctrl_c().await;
    }))
}

/// The daemon proper: start the proxy + redirector + IPC server and serve until
/// `shutdown` resolves (Ctrl-C in the foreground, the SCM Stop control under a
/// Windows service), then restore DNS. Shared by both entry paths.
pub(crate) async fn run_daemon(shutdown: impl std::future::Future<Output = ()>) -> Result<()> {
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

    let server = ipc::serve(controller.clone());
    tokio::pin!(shutdown);
    tokio::select! {
        res = server => res?,
        _ = &mut shutdown => {
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
