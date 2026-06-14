//! Local IPC between the GUI (client) and the service (server).
//!
//! Transport: `interprocess` local sockets — a named pipe on Windows, a Unix
//! domain socket elsewhere. Framing: a `u32` big-endian length prefix followed
//! by JSON. Commands are one-shot connections; status is streamed over a single
//! long-lived `Subscribe` connection.

use std::io;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use interprocess::local_socket::tokio::prelude::*;
#[allow(unused_imports)]
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tracing::{info, warn};
use voidns_proto::{Command, Event, Status};

use crate::Controller;

/// Resolve the IPC endpoint name. `VOIDNS_SOCK` overrides the default (handy
/// for unprivileged dev/tests).
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

// --- framing ---

async fn write_msg<W, T>(w: &mut W, msg: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let buf = serde_json::to_vec(msg)?;
    w.write_u32(buf.len() as u32).await?;
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

async fn read_msg<R, T>(r: &mut R) -> Result<Option<T>>
where
    R: AsyncReadExt + Unpin,
    T: DeserializeOwned,
{
    let len = match r.read_u32().await {
        Ok(n) => n as usize,
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

// --- server ---

/// Run the IPC server until the listener errors. Each connection is handled
/// concurrently against the shared [`Controller`].
pub async fn serve(controller: Arc<Mutex<Controller>>) -> Result<()> {
    #[cfg(not(windows))]
    prepare_unix_path();

    let listener = ListenerOptions::new()
        .name(endpoint_name()?)
        .create_tokio()?;

    // Let the unprivileged GUI (running as the logged-in user) connect to the
    // root-owned socket. The socket lives in a root-owned dir, so this does not
    // expose it beyond local users. Peer-credential hardening is a follow-up.
    #[cfg(not(windows))]
    relax_socket_perms();

    info!("IPC server listening");

    loop {
        let stream = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "accept failed");
                continue;
            }
        };
        let controller = controller.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, controller).await {
                warn!(error = %e, "connection closed with error");
            }
        });
    }
}

#[cfg(not(windows))]
fn prepare_unix_path() {
    use std::path::Path;
    let path = match std::env::var("VOIDNS_SOCK") {
        Ok(s) => s,
        Err(_) => voidns_proto::IPC_SOCK_PATH.to_string(),
    };
    if let Some(parent) = Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Remove a stale socket so bind() can re-create it.
    let _ = std::fs::remove_file(&path);
}

#[cfg(not(windows))]
fn relax_socket_perms() {
    use std::os::unix::fs::PermissionsExt;
    let path =
        std::env::var("VOIDNS_SOCK").unwrap_or_else(|_| voidns_proto::IPC_SOCK_PATH.to_string());
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
}

async fn handle_conn(stream: LocalSocketStream, controller: Arc<Mutex<Controller>>) -> Result<()> {
    let (mut r, mut w) = tokio::io::split(stream);
    let mut subscription: Option<tokio::sync::broadcast::Receiver<Status>> = None;

    loop {
        tokio::select! {
            incoming = read_msg::<_, Command>(&mut r) => {
                let cmd = match incoming? {
                    Some(c) => c,
                    None => break, // peer closed
                };
                let reply = dispatch(cmd, &controller, &mut subscription).await;
                write_msg(&mut w, &reply).await?;
            }
            // Forward broadcast status updates if subscribed.
            update = recv_opt(&mut subscription) => {
                if let Some(status) = update {
                    write_msg(&mut w, &Event::Status(status)).await?;
                }
            }
        }
    }
    Ok(())
}

async fn dispatch(
    cmd: Command,
    controller: &Arc<Mutex<Controller>>,
    subscription: &mut Option<tokio::sync::broadcast::Receiver<Status>>,
) -> Event {
    match cmd {
        Command::Ping => Event::Pong,
        Command::GetStatus => Event::Status(controller.lock().await.status()),
        Command::Subscribe => {
            let guard = controller.lock().await;
            *subscription = Some(guard.subscribe());
            Event::Status(guard.status())
        }
        Command::Connect { upstream } => {
            Event::Status(controller.lock().await.connect(upstream).await)
        }
        Command::Disconnect => Event::Status(controller.lock().await.disconnect().await),
    }
}

/// Resolve to the next broadcast item, or never (if not subscribed).
async fn recv_opt(sub: &mut Option<tokio::sync::broadcast::Receiver<Status>>) -> Option<Status> {
    match sub {
        Some(rx) => rx.recv().await.ok(),
        None => std::future::pending().await,
    }
}

// --- client ---

/// Send one command and read the single reply (opens and closes a connection).
pub async fn one_shot(cmd: &Command) -> Result<Event> {
    let mut stream = LocalSocketStream::connect(endpoint_name()?).await?;
    write_msg(&mut stream, cmd).await?;
    read_msg::<_, Event>(&mut stream)
        .await?
        .ok_or_else(|| anyhow!("service closed connection without replying"))
}

/// A long-lived status subscription.
pub struct Subscription {
    stream: LocalSocketStream,
}

impl Subscription {
    pub async fn open() -> Result<Self> {
        let mut stream = LocalSocketStream::connect(endpoint_name()?).await?;
        write_msg(&mut stream, &Command::Subscribe).await?;
        Ok(Self { stream })
    }

    /// Await the next status update. `Ok(None)` when the service disconnects.
    pub async fn next(&mut self) -> Result<Option<Status>> {
        loop {
            match read_msg::<_, Event>(&mut self.stream).await? {
                Some(Event::Status(s)) => return Ok(Some(s)),
                Some(_) => continue,
                None => return Ok(None),
            }
        }
    }
}
