//! Shared IPC protocol between the unprivileged GUI and the privileged
//! `voidns-service`. Both sides serialize these types as newline-free,
//! length-delimited JSON frames (see `voidns-core::ipc`).

use serde::{Deserialize, Serialize};

/// Default loopback port the proxy binds. 53 in production (requires privilege);
/// overridable for unprivileged dev/tests.
pub const DEFAULT_PROXY_PORT: u16 = 53;

/// Fixed IPC endpoint name. On Windows this becomes a named pipe
/// (`\\.\pipe\voidns-service`); on Unix a filesystem socket path.
pub const IPC_PIPE_NAME: &str = "voidns-service";
pub const IPC_SOCK_PATH: &str = "/run/voidns/control.sock";

/// Upstream DoH resolver selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpstreamSel {
    /// The voidns hosted DoH endpoint. Carries a bootstrap IP so the proxy
    /// never has to recursively resolve its own upstream (RFC 8484 §10).
    Voidns,
    #[default]
    Cloudflare,
    Google,
    Quad9,
    /// Arbitrary DoH endpoint. `ip` is the hardcoded bootstrap address,
    /// `hostname` the TLS SNI / cert name, `path` the RFC 8484 query path.
    Custom {
        ip: String,
        hostname: String,
        path: String,
    },
}

impl UpstreamSel {
    /// Short human label for the UI.
    pub fn label(&self) -> &str {
        match self {
            UpstreamSel::Voidns => "voidns",
            UpstreamSel::Cloudflare => "Cloudflare",
            UpstreamSel::Google => "Google",
            UpstreamSel::Quad9 => "Quad9",
            UpstreamSel::Custom { hostname, .. } => hostname,
        }
    }
}

/// Commands sent GUI -> service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Spin up the proxy and redirect system DNS to it.
    Connect { upstream: UpstreamSel },
    /// Restore DNS and stop the proxy.
    Disconnect,
    /// Request a one-shot status reply.
    GetStatus,
    /// Ask the service to stream `Event::Status` on every state change.
    Subscribe,
    /// Liveness check.
    Ping,
}

/// Events / replies sent service -> GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Status(Status),
    Pong,
    /// A command was rejected or failed.
    Error {
        message: String,
    },
}

/// Connection lifecycle state mirrored by the GUI button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Full status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub state: ConnState,
    /// Active upstream label when connected.
    pub upstream: Option<String>,
    /// Loopback address:port the proxy is listening on.
    pub listen: Option<String>,
    /// Last error message, if `state == Error`.
    pub error: Option<String>,
}

impl Status {
    pub fn disconnected() -> Self {
        Status {
            state: ConnState::Disconnected,
            upstream: None,
            listen: None,
            error: None,
        }
    }
}
