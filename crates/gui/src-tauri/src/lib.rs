// VoidNS Client — Rust core (Tauri v2).
//
// Split-privilege design (mirrors AmneziaVPN): the GUI is UNPRIVILEGED. It never
// touches DNS or binds port 53 itself — it talks to the privileged background
// `voidns-service` (installed once as a root systemd unit) over a local socket
// (see `ipc`) and just forwards Connect/Disconnect/GetStatus, streaming status
// back to the UI. That root service does the privileged DNS work, so the user
// runs the GUI without sudo — exactly like Amnezia's daemon.

mod ipc;

use std::io;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use voidns_proto::{Command, Event, Status, UpstreamSel};

#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// На Linux отключает dmabuf-рендер webkit2gtk: на части GPU/драйверов он даёт
/// белое окно/артефакты. No-op на других ОС и если переменная задана снаружи.
/// Вызывать в начале `run()` — ДО инициализации webkit (создания окна).
fn disable_webkit_dmabuf() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}

/// Map a service reply to a command result. `Event::Error` becomes `Err`.
fn into_status(reply: io::Result<Event>) -> Result<Status, String> {
    match reply {
        Ok(Event::Status(s)) => Ok(s),
        Ok(Event::Error { message }) => Err(message),
        Ok(_) => Err("unexpected reply from service".into()),
        Err(e) => Err(format!("voidns-service unreachable: {e}")),
    }
}

#[tauri::command]
async fn connect(upstream: UpstreamSel) -> Result<Status, String> {
    into_status(ipc::one_shot(Command::Connect { upstream }).await)
}

#[tauri::command]
async fn disconnect() -> Result<Status, String> {
    into_status(ipc::one_shot(Command::Disconnect).await)
}

#[tauri::command]
async fn get_status() -> Result<Status, String> {
    into_status(ipc::one_shot(Command::GetStatus).await)
}

/// System tray. Icon is the frontend site's favicon (icons/tray.png). Left
/// click toggles the window (where the platform delivers the event); the menu
/// (Show / Quit) is the portable fallback — on Linux/AppIndicator only the menu
/// is guaranteed to fire.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(tauri::include_image!("icons/tray.png"))
        .icon_as_template(false)
        .tooltip("VoidNS Client")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(true) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    disable_webkit_dmabuf();
    tauri::Builder::default()
        .setup(|app| {
            build_tray(app)?;

            // Stream live status from the service to the webview.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(ipc::subscribe_forever(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            connect,
            disconnect,
            get_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
