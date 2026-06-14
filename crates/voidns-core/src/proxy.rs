//! Local DNS → DoH proxy.
//!
//! Accepts plain UDP+TCP DNS on a loopback socket and forwards every query over
//! DNS-over-HTTPS (RFC 8484) to the selected upstream. Built on hickory-dns
//! 0.26: a [`Catalog`] whose root zone is a [`ForwardZoneHandler`] (DoH
//! resolver). Start/stop maps onto Connect/Disconnect.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use hickory_server::proto::rr::{LowerName, Name};
use hickory_server::resolver::config::{NameServerConfig, ResolverOpts, CLOUDFLARE, GOOGLE, QUAD9};
use hickory_server::store::forwarder::{ForwardConfig, ForwardZoneHandler};
use hickory_server::zone_handler::{Catalog, ZoneHandler};
use hickory_server::Server;
use tokio::net::{TcpListener, UdpSocket};
use tracing::info;
use voidns_proto::UpstreamSel;

/// Idle timeout for TCP DNS connections.
const TCP_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-connection outgoing response buffer.
const TCP_RESPONSE_BUF: usize = 65_535;

// --- voidns hosted DoH endpoint (placeholder bootstrap until provisioned) ---
// The IP is hardcoded so the proxy never recursively resolves its own upstream
// (RFC 8484 §10 bootstrap problem). Replace with the real voidns anycast IP.
const VOIDNS_DOH_IP: &str = "1.1.1.1";
const VOIDNS_DOH_HOST: &str = "cloudflare-dns.com";
const VOIDNS_DOH_PATH: &str = "/dns-query";

/// A running DoH proxy. Dropping or calling [`DohProxy::stop`] tears down the
/// listeners and releases the bound port.
pub struct DohProxy {
    server: Server<Catalog>,
    listen: SocketAddr,
}

impl DohProxy {
    /// Bind `listen` (UDP+TCP) and start forwarding to `upstream`.
    pub async fn start(listen: SocketAddr, upstream: &UpstreamSel) -> Result<Self> {
        let name_servers = upstream_name_servers(upstream)?;

        let mut options = ResolverOpts::default();
        options.cache_size = 1024;
        options.positive_min_ttl = Some(Duration::from_secs(5));
        options.positive_max_ttl = Some(Duration::from_secs(3600));
        options.negative_min_ttl = Some(Duration::from_secs(5));
        options.negative_max_ttl = Some(Duration::from_secs(300));
        options.try_tcp_on_error = true;
        options.num_concurrent_reqs = 2;

        let forward = ForwardConfig {
            name_servers,
            options: Some(options),
        };

        let zone = ForwardZoneHandler::builder_tokio(forward)
            .with_origin(Name::root())
            .build()
            .map_err(|e| anyhow!("failed to build DoH forwarder: {e}"))?;

        // Root zone "." → catch-all forwarder.
        let mut catalog = Catalog::new();
        catalog.upsert(
            LowerName::from(Name::root()),
            vec![Arc::new(zone) as Arc<dyn ZoneHandler>],
        );

        let mut server = Server::new(catalog);

        let udp = UdpSocket::bind(listen)
            .await
            .with_context(|| format!("bind UDP {listen}"))?;
        server.register_socket(udp);

        let tcp = TcpListener::bind(listen)
            .await
            .with_context(|| format!("bind TCP {listen}"))?;
        server.register_listener(tcp, TCP_TIMEOUT, TCP_RESPONSE_BUF);

        info!(%listen, upstream = upstream.label(), "DoH proxy listening");
        Ok(Self { server, listen })
    }

    pub fn listen(&self) -> SocketAddr {
        self.listen
    }

    /// Gracefully stop the listeners and release the port.
    pub async fn stop(mut self) -> Result<()> {
        self.server
            .shutdown_gracefully()
            .await
            .map_err(|e| anyhow!("proxy shutdown error: {e}"))?;
        info!(listen = %self.listen, "DoH proxy stopped");
        Ok(())
    }
}

/// Translate an [`UpstreamSel`] into hickory DoH name-server configs.
fn upstream_name_servers(upstream: &UpstreamSel) -> Result<Vec<NameServerConfig>> {
    let servers = match upstream {
        UpstreamSel::Cloudflare => CLOUDFLARE.https().collect(),
        UpstreamSel::Google => GOOGLE.https().collect(),
        UpstreamSel::Quad9 => QUAD9.https().collect(),
        UpstreamSel::Voidns => {
            let ip: IpAddr = VOIDNS_DOH_IP.parse().context("voidns DoH IP")?;
            vec![NameServerConfig::https(
                ip,
                Arc::from(VOIDNS_DOH_HOST),
                Some(Arc::from(VOIDNS_DOH_PATH)),
            )]
        }
        UpstreamSel::Custom { ip, hostname, path } => {
            let ip: IpAddr = ip.parse().context("custom DoH bootstrap IP")?;
            vec![NameServerConfig::https(
                ip,
                Arc::from(hostname.as_str()),
                Some(Arc::from(path.as_str())),
            )]
        }
    };
    Ok(servers)
}
