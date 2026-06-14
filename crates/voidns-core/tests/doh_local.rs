//! Hermetic DoH: spin up the vendored local mock DoH resolver (reused from the
//! backend browser-doh suite) and prove our hickory DoH client stack resolves
//! through it fully offline — trusting the mock's self-signed cert via a
//! programmatic `RootCertStore` (prod stays on webpki-roots, untouched).
//!
//! Complements `proxy_doh.rs` (full DohProxy against real Cloudflare): this one
//! is network-free and runs identically on Linux/Windows/macOS in CI. The CI job
//! runs `npm ci` in tests/doh-mock first; locally the test skips gracefully if
//! `node` or the mock deps are missing (same pattern as proxy_doh).

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::Resolver;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

fn mock_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/doh-mock")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resolves_via_local_mock_doh() {
    let dir = mock_dir();

    // Preconditions — skip (don't fail) when the node mock can't run, mirroring
    // proxy_doh.rs's network-unavailable skip. CI installs node + deps so it runs.
    if Command::new("node")
        .arg("--version")
        .output()
        .await
        .is_err()
    {
        eprintln!("SKIP: `node` not available");
        return;
    }
    if !dir.join("node_modules/dns-packet").exists() {
        eprintln!(
            "SKIP: mock deps not installed — run `npm ci` in {}",
            dir.display()
        );
        return;
    }

    // 1. Self-signed cert for `localhost` (the SNI the resolver validates).
    let certified =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).expect("generate cert");
    let cert_der = certified.cert.der().clone();

    let tmp = std::env::temp_dir().join(format!("voidns-doh-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let cert_path = tmp.join("cert.pem");
    let key_path = tmp.join("key.pem");
    std::fs::write(&cert_path, certified.cert.pem()).unwrap();
    std::fs::write(&key_path, certified.key_pair.serialize_pem()).unwrap();

    // 2. Start the mock on an ephemeral port; read the port it prints.
    let mut child = Command::new("node")
        .arg("mock-doh.mjs")
        .arg(&cert_path)
        .arg(&key_path)
        .arg("0")
        .current_dir(&dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn node mock");

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let port: u16 = match tokio::time::timeout(Duration::from_secs(15), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(p) = line.strip_prefix("PORT=") {
                return p.trim().parse::<u16>().ok();
            }
        }
        None
    })
    .await
    {
        Ok(Some(p)) => p,
        _ => {
            let _ = child.kill().await;
            panic!("mock DoH did not report a port");
        }
    };

    // 3. rustls client config trusting only the mock's cert (programmatic root).
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).expect("add mock root");
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("tls versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![b"h2".to_vec()];

    // 4. hickory resolver pointed at 127.0.0.1:<mock> over DoH, custom-trusted.
    let mut ns = NameServerConfig::https(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        Arc::from("localhost"),
        Some(Arc::from("/dns-query")),
    );
    ns.connections[0].port = port;
    let config = ResolverConfig::from_parts(None, vec![], vec![ns]);
    let resolver = Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .with_tls_config(tls)
        .build()
        .expect("build resolver");

    // 5. Resolve through the mock — it answers every name with 127.0.0.1.
    let result = tokio::time::timeout(Duration::from_secs(10), resolver.lookup_ip("example.com."))
        .await
        .expect("lookup timed out")
        .expect("lookup failed");

    let addrs: Vec<IpAddr> = result.iter().collect();
    let _ = child.kill().await;
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(
        addrs.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)),
        "expected mock to answer 127.0.0.1, got {addrs:?}"
    );
}
