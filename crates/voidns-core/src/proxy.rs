//! Local DNS → DoH proxy.
//!
//! Accepts plain UDP+TCP DNS on a loopback socket and forwards every query over
//! DNS-over-HTTPS (RFC 8484) to the selected upstream. Built on hickory-dns
//! 0.26: a [`Catalog`] whose root zone is a [`VerifiedForwarder`] (DoH resolver
//! with an offline rustls trust store). Start/stop maps onto Connect/Disconnect.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use hickory_server::proto::rr::{LowerName, Name, RecordType, TSigResponseContext};
use hickory_server::resolver::config::{
    NameServerConfig, ResolveHosts, ResolverConfig, ResolverOpts, CLOUDFLARE, GOOGLE, QUAD9,
};
use hickory_server::resolver::net::runtime::TokioRuntimeProvider;
use hickory_server::resolver::Resolver;
use hickory_server::server::{Request, RequestInfo};
use hickory_server::zone_handler::{
    AuthLookup, AxfrPolicy, Catalog, LookupControlFlow, LookupError, LookupOptions, ZoneHandler,
    ZoneType,
};
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
        // A forwarder must echo intermediate CNAMEs (RFC 1034 §4.3.2) and must
        // not consult the local hosts file. (Matches hickory's ForwardZoneHandler.)
        options.preserve_intermediates = true;
        options.use_hosts_file = ResolveHosts::Never;

        let zone = build_forward_zone(name_servers, options)?;

        // Root zone "." → catch-all forwarder.
        let mut catalog = Catalog::new();
        catalog.upsert(LowerName::from(Name::root()), vec![zone]);

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
        UpstreamSel::Custom {
            ip,
            hostname,
            path,
            port,
        } => {
            let ip: IpAddr = ip.parse().context("custom DoH bootstrap IP")?;
            let mut ns = NameServerConfig::https(
                ip,
                Arc::from(hostname.as_str()),
                Some(Arc::from(path.as_str())),
            );
            // `NameServerConfig::https` defaults to 443; override so the mock
            // DoH server (and any non-standard endpoint) is reachable.
            for conn in &mut ns.connections {
                conn.port = *port;
            }
            vec![ns]
        }
    };
    Ok(servers)
}

/// Build the root "." forward zone: a DoH forwarder whose upstream TLS uses an
/// explicit, offline rustls trust store (see [`build_client_config`]). This is
/// our own minimal equivalent of hickory's `ForwardZoneHandler` — needed only
/// because that type builds its resolver internally and gives no hook to inject
/// a custom `rustls::ClientConfig`.
fn build_forward_zone(
    name_servers: Vec<NameServerConfig>,
    options: ResolverOpts,
) -> Result<Arc<dyn ZoneHandler>> {
    let client_config = build_client_config()?;
    let resolver_config = ResolverConfig::from_parts(None, vec![], name_servers);

    let mut builder =
        Resolver::builder_with_config(resolver_config, TokioRuntimeProvider::default());
    builder = builder.with_tls_config(client_config);
    *builder.options_mut() = options;
    let resolver = builder
        .build()
        .map_err(|e| anyhow!("failed to build DoH resolver: {e}"))?;

    Ok(Arc::new(VerifiedForwarder {
        origin: LowerName::from(Name::root()),
        resolver,
    }))
}

/// Default path for an admin-supplied extra CA bundle (private/internal DoH
/// endpoints). Read in addition to the built-in roots if present; only a
/// privileged user can write it, same as the service itself.
fn default_extra_ca_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\ProgramData"));
        base.join("VoidNS").join("extra-ca.pem")
    }
    #[cfg(not(windows))]
    {
        std::path::PathBuf::from("/etc/voidns/extra-ca.pem")
    }
}

/// Build the rustls client config for upstream DoH: the offline Mozilla
/// webpki-roots bundle, plus an optional extra CA. The extra CA comes from
/// `VOIDNS_EXTRA_CA_FILE` if set (an error if it can't be read — it was asked
/// for explicitly), otherwise from [`default_extra_ca_path`] if that file
/// happens to exist. Offline by design — see the note in Cargo.toml on why the
/// OS verifier would deadlock once the system DNS is redirected to us.
fn build_client_config() -> Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let (ca_path, required) = match std::env::var_os("VOIDNS_EXTRA_CA_FILE") {
        Some(p) => (std::path::PathBuf::from(p), true),
        None => (default_extra_ca_path(), false),
    };
    match std::fs::read(&ca_path) {
        Ok(pem) => {
            let mut added = 0usize;
            for cert in rustls_pemfile::certs(&mut &pem[..]) {
                let cert = cert.context("parse extra CA PEM")?;
                roots.add(cert).context("add extra CA to root store")?;
                added += 1;
            }
            info!(path = ?ca_path, added, "trusting extra CA(s) for upstream DoH");
        }
        Err(e) if required => {
            return Err(anyhow!("read VOIDNS_EXTRA_CA_FILE {ca_path:?}: {e}"));
        }
        Err(_) => {} // No admin CA at the default path — the common case.
    }

    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("rustls protocol versions")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(config)
}

/// Minimal root-zone forwarder: hand every query to an upstream DoH resolver.
/// Mirrors hickory's `ForwardZoneHandler` zone-handler impl (we only need a
/// custom one so the resolver can carry our [`build_client_config`] TLS config).
struct VerifiedForwarder {
    origin: LowerName,
    resolver: Resolver<TokioRuntimeProvider>,
}

#[async_trait::async_trait]
impl ZoneHandler for VerifiedForwarder {
    fn zone_type(&self) -> ZoneType {
        ZoneType::External
    }

    fn axfr_policy(&self) -> AxfrPolicy {
        AxfrPolicy::Deny
    }

    fn origin(&self) -> &LowerName {
        &self.origin
    }

    async fn lookup(
        &self,
        name: &LowerName,
        rtype: RecordType,
        _request_info: Option<&RequestInfo<'_>>,
        _lookup_options: LookupOptions,
    ) -> LookupControlFlow<AuthLookup> {
        // Drop the FQDN flag so the upstream resolver does not append a search
        // domain (matches hickory's forwarder).
        let mut name: Name = name.clone().into();
        name.set_fqdn(false);

        use LookupControlFlow::*;
        match self.resolver.lookup(name, rtype).await {
            Ok(lookup) => Continue(Ok(AuthLookup::from(lookup))),
            Err(e) => Continue(Err(LookupError::from(e))),
        }
    }

    async fn search(
        &self,
        request: &Request,
        lookup_options: LookupOptions,
    ) -> (LookupControlFlow<AuthLookup>, Option<TSigResponseContext>) {
        let request_info = match request.request_info() {
            Ok(info) => info,
            Err(e) => return (LookupControlFlow::Break(Err(e)), None),
        };
        (
            self.lookup(
                request_info.query.name(),
                request_info.query.query_type(),
                Some(&request_info),
                lookup_options,
            )
            .await,
            None,
        )
    }

    async fn nsec_records(
        &self,
        _name: &LowerName,
        _lookup_options: LookupOptions,
    ) -> LookupControlFlow<AuthLookup> {
        LookupControlFlow::Continue(Err(LookupError::from(std::io::Error::other(
            "NSEC records are unimplemented for the forwarder",
        ))))
    }
}
