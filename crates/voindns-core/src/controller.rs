//! Connect/Disconnect state machine. Owns the proxy and the redirector, and
//! broadcasts every status change to subscribers (the IPC layer forwards these
//! to the GUI).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Error;
use tokio::sync::broadcast;
use tokio::task::block_in_place;
use voindns_proto::{ConnState, Status, UpstreamSel};

use crate::proxy::DohProxy;
use crate::redirect::{self, DnsRedirector};

pub struct Controller {
    status: Status,
    proxy: Option<DohProxy>,
    redirector: Box<dyn DnsRedirector>,
    port: u16,
    tx: broadcast::Sender<Status>,
}

impl Controller {
    pub fn new(port: u16) -> anyhow::Result<Self> {
        let (tx, _) = broadcast::channel(16);
        Ok(Self {
            status: Status::disconnected(),
            proxy: None,
            redirector: redirect::new_redirector()?,
            port,
            tx,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Status> {
        self.tx.subscribe()
    }

    pub fn status(&self) -> Status {
        self.status.clone()
    }

    fn set(&mut self, status: Status) {
        self.status = status.clone();
        let _ = self.tx.send(status);
    }

    pub async fn connect(&mut self, upstream: UpstreamSel) -> Status {
        if matches!(
            self.status.state,
            ConnState::Connected | ConnState::Connecting
        ) {
            return self.status.clone();
        }
        let label = upstream.label().to_string();
        self.set(Status {
            state: ConnState::Connecting,
            upstream: Some(label.clone()),
            listen: None,
            error: None,
        });

        let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);

        // 1. Start the local DoH proxy.
        let proxy = match DohProxy::start(listen, &upstream).await {
            Ok(p) => p,
            Err(e) => {
                self.set(error_status(&e));
                return self.status.clone();
            }
        };

        // 2. Redirect system DNS to it (blocking work off the async path).
        let redirect_result = block_in_place(|| self.redirector.apply(listen.ip()));
        if let Err(e) = redirect_result {
            let _ = proxy.stop().await;
            self.set(error_status(&e));
            return self.status.clone();
        }
        let _ = block_in_place(|| self.redirector.flush_cache());

        self.proxy = Some(proxy);
        self.set(Status {
            state: ConnState::Connected,
            upstream: Some(label),
            listen: Some(listen.to_string()),
            error: None,
        });
        self.status.clone()
    }

    pub async fn disconnect(&mut self) -> Status {
        // Restore DNS *before* killing the proxy so there is no window where the
        // system points at a dead :53.
        let _ = block_in_place(|| self.redirector.restore());
        if let Some(proxy) = self.proxy.take() {
            let _ = proxy.stop().await;
        }
        let _ = block_in_place(|| self.redirector.flush_cache());
        self.set(Status::disconnected());
        self.status.clone()
    }
}

fn error_status(e: &Error) -> Status {
    Status {
        state: ConnState::Error,
        upstream: None,
        listen: None,
        error: Some(format!("{e:#}")),
    }
}
