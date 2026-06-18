//! Windows Service Control Manager (SCM) integration.
//!
//! When the daemon is launched by the SCM (`sc start voidns`, or the GUI's NSIS
//! installer hook), Windows expects the process to speak the service-control
//! protocol — register a control handler and report `Running` — within ~30s, or
//! it fails the start with error 1053. A plain console binary never does this,
//! so without this module `sc start` times out and the service never actually
//! runs. Here we register the handler, report `Running`, then drive the same
//! async daemon (`crate::run_daemon`) until the SCM sends Stop/Shutdown.
//!
//! The SAME binary is also run in the foreground (dev, or `voidns-service run`
//! from a console): `service_dispatcher::start` returns 1063
//! (ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) when we were not started by the
//! SCM, which we treat as "run in the foreground instead".

use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

const SERVICE_NAME: &str = "voidns";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

windows_service::define_windows_service!(ffi_service_main, service_main);

/// Try to run under the SCM. `Ok(true)` means we ran as a service and have now
/// stopped; `Ok(false)` means the process was not started by the SCM, so the
/// caller should run in the foreground instead.
pub fn try_run_as_service() -> Result<bool> {
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(()) => Ok(true),
        // ERROR_FAILED_SERVICE_CONTROLLER_CONNECT — not launched by the SCM.
        Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1063) => Ok(false),
        Err(e) => Err(anyhow!("service dispatcher failed: {e}")),
    }
}

/// SCM entry point (invoked on a background thread by the generated FFI shim).
fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service() {
        tracing::error!(error = %format!("{e:#}"), "voidns service stopped with error");
    }
}

fn run_service() -> Result<()> {
    // Bridge SCM Stop/Shutdown (delivered on another thread) into the daemon.
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    let event_handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = shutdown_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    let report = |state, accept| {
        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
    };

    // Report Running right away so the SCM start request succeeds, then serve.
    report(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
    )?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(crate::run_daemon(async move {
        let _ = tokio::task::spawn_blocking(move || {
            let _ = shutdown_rx.recv();
        })
        .await;
    }));

    report(ServiceState::Stopped, ServiceControlAccept::empty())?;
    result
}
