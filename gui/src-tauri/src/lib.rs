//! Tauri shell for the voindns GUI. Exposes connect/disconnect/get_status
//! commands that proxy to the privileged service over local IPC, and forwards
//! the service's status stream to the webview as a `status` event.

use tauri::{Emitter, Manager};
use voindns_core::ipc;
use voindns_proto::{Command, Event, Status, UpstreamSel};

fn into_status(reply: anyhow::Result<Event>) -> Result<Status, String> {
    match reply {
        Ok(Event::Status(s)) => Ok(s),
        Ok(Event::Error { message }) => Err(message),
        Ok(_) => Err("unexpected reply from service".into()),
        Err(e) => Err(format!("service unavailable: {e}")),
    }
}

#[tauri::command]
async fn get_status() -> Result<Status, String> {
    into_status(ipc::one_shot(&Command::GetStatus).await)
}

#[tauri::command]
async fn connect(upstream: UpstreamSel) -> Result<Status, String> {
    into_status(ipc::one_shot(&Command::Connect { upstream }).await)
}

#[tauri::command]
async fn disconnect() -> Result<Status, String> {
    into_status(ipc::one_shot(&Command::Disconnect).await)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Single instance must be registered first.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            // Maintain a live status subscription; re-connect if the service
            // restarts or isn't up yet.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    if let Ok(mut sub) = ipc::Subscription::open().await {
                        while let Ok(Some(status)) = sub.next().await {
                            let _ = handle.emit("status", status);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_status, connect, disconnect])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
