//! `voidns` — the command-line client.
//!
//! This is the terminal twin of the GUI: it sends the *exact same* IPC commands
//! (`Connect` / `Disconnect` / `GetStatus` / `Ping`) over the *exact same*
//! framing to the privileged `voidns-service`, and depends only on the shared
//! `voidns-proto` types — NOT on `voidns-core` (no proxy/redirect/privileged
//! deps). The framing below is byte-for-byte identical to the GUI's
//! `crates/gui/src-tauri/src/ipc.rs`, so "the CLI talks to the service 1:1 like
//! the GUI does" is literally true.
//!
//! Usage:
//!   voidns connect <cloudflare|google|quad9|voidns|custom <ip> <host> <path> <port>>
//!   voidns disconnect
//!   voidns status
//!   voidns ping
//!
//! `VOIDNS_SOCK` overrides the endpoint (handy for unprivileged dev/tests and to
//! match a service started with a non-default socket, e.g. the macOS daemon).

use std::io;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use interprocess::local_socket::tokio::prelude::*;
#[allow(unused_imports)]
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use voidns_proto::{Command, ConnState, Event, UpstreamSel};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match dispatch(&args).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn dispatch(args: &[String]) -> Result<ExitCode> {
    let cmd = match args.get(1).map(String::as_str) {
        Some("connect") => Command::Connect {
            upstream: parse_upstream(&args[2..])?,
        },
        Some("disconnect") => Command::Disconnect,
        Some("status") => Command::GetStatus,
        Some("ping") => Command::Ping,
        Some("-h") | Some("--help") | Some("help") | None => {
            print_usage();
            return Ok(ExitCode::SUCCESS);
        }
        Some(other) => bail!("unknown command `{other}`\n\n{}", USAGE),
    };
    run(cmd).await
}

/// Send one command, print the reply, and map it to a process exit code so
/// scripts can gate on it (non-zero on error, or on a connect that did not reach
/// `Connected`). Mirrors `voidns-service`'s own `ctl`.
async fn run(cmd: Command) -> Result<ExitCode> {
    let connecting = matches!(cmd, Command::Connect { .. });
    match one_shot(&cmd).await? {
        Event::Status(s) => {
            println!(
                "state={:?} upstream={} listen={} error={}",
                s.state,
                s.upstream.as_deref().unwrap_or("-"),
                s.listen.as_deref().unwrap_or("-"),
                s.error.as_deref().unwrap_or("-"),
            );
            let bad = matches!(s.state, ConnState::Error)
                || (connecting && s.state != ConnState::Connected);
            Ok(if bad {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Event::Pong => {
            println!("pong");
            Ok(ExitCode::SUCCESS)
        }
        Event::Error { message } => {
            eprintln!("error: {message}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Parse a CLI upstream spec into an [`UpstreamSel`] (identical grammar to the
/// `voidns-service connect` control command).
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
        _ => bail!("connect: missing upstream\n\n{}", USAGE),
    }
}

const USAGE: &str = "usage:\n  \
    voidns connect <cloudflare|google|quad9|voidns|custom <ip> <hostname> <path> <port>>\n  \
    voidns disconnect\n  \
    voidns status\n  \
    voidns ping";

fn print_usage() {
    println!("{USAGE}");
}

// --- IPC client (byte-for-byte identical to the GUI's src/ipc.rs) ------------

/// Resolve the IPC endpoint. `VOIDNS_SOCK` overrides the default.
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

/// Send one command and read the single reply (opens and closes a connection).
async fn one_shot(cmd: &Command) -> Result<Event> {
    let mut stream = LocalSocketStream::connect(endpoint_name()?)
        .await
        .context("voidns-service unreachable (is it installed and running?)")?;
    write_msg(&mut stream, cmd).await?;
    read_msg::<_, Event>(&mut stream)
        .await?
        .context("service closed connection without replying")
}
