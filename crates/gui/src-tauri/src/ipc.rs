//! Minimal IPC client to `voidns-service`.
//!
//! Mirrors `voidns-core::ipc` framing exactly — a `u32` big-endian length
//! prefix followed by JSON — but is re-implemented here so the unprivileged GUI
//! depends only on the shared `voidns-proto` types, NOT on the privileged core
//! (which would drag in the DoH proxy, hickory, zbus/nix, etc.).
//!
//! One-shot commands open and close a connection; status is streamed over a
//! single long-lived `Subscribe` connection and re-emitted to the webview as
//! the `voidns://status` event.

use std::io;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::*;
#[allow(unused_imports)]
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use voidns_proto::{Command, Event};

/// Webview event carrying every service status change.
pub const STATUS_EVENT: &str = "voidns://status";

/// Resolve the IPC endpoint. `VOIDNS_SOCK` overrides the default (handy for
/// unprivileged dev: run the service with a writable socket path + high port).
fn endpoint_name() -> io::Result<Name<'static>> {
    let name: &'static str = match std::env::var("VOIDNS_SOCK") {
        Ok(s) => Box::leak(s.into_boxed_str()),
        Err(_) => default_name(),
    };
    #[cfg(windows)]
    {
        name.to_ns_name::<GenericNamespaced>()
    }
    #[cfg(not(windows))]
    {
        name.to_fs_name::<GenericFilePath>()
    }
}

fn default_name() -> &'static str {
    #[cfg(windows)]
    {
        voidns_proto::IPC_PIPE_NAME
    }
    #[cfg(not(windows))]
    {
        voidns_proto::IPC_SOCK_PATH
    }
}

// --- framing (identical to voidns-core::ipc) ---

async fn write_msg<W, T>(w: &mut W, msg: &T) -> io::Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let buf = serde_json::to_vec(msg).map_err(io::Error::other)?;
    w.write_u32(buf.len() as u32).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

async fn read_msg<R, T>(r: &mut R) -> io::Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: DeserializeOwned,
{
    let len = match r.read_u32().await {
        Ok(n) => n as usize,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    let v = serde_json::from_slice(&buf).map_err(io::Error::other)?;
    Ok(Some(v))
}

// --- client ---

/// Send one command and read the single reply (opens and closes a connection).
pub async fn one_shot(cmd: Command) -> io::Result<Event> {
    let mut stream = LocalSocketStream::connect(endpoint_name()?).await?;
    write_msg(&mut stream, &cmd).await?;
    read_msg::<_, Event>(&mut stream)
        .await?
        .ok_or_else(|| io::Error::other("service closed connection without replying"))
}

/// Open a long-lived `Subscribe` connection and emit every status change to the
/// webview. Reconnects forever — the service may not be up yet, or may restart.
pub async fn subscribe_forever(app: AppHandle) {
    loop {
        // Errors are expected when the service is down; just back off and retry.
        let _ = subscribe_once(&app).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn subscribe_once(app: &AppHandle) -> io::Result<()> {
    let mut stream = LocalSocketStream::connect(endpoint_name()?).await?;
    write_msg(&mut stream, &Command::Subscribe).await?;
    loop {
        match read_msg::<_, Event>(&mut stream).await? {
            Some(Event::Status(status)) => {
                let _ = app.emit(STATUS_EVENT, status);
            }
            Some(_) => {}
            None => return Ok(()), // service closed the stream
        }
    }
}
