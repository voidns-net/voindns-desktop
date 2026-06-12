//! End-to-end: start the local proxy, send it a real UDP DNS query, and assert
//! it forwards over DoH and returns an answer. Requires outbound HTTPS; skips
//! gracefully if the network is unavailable.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use voindns_core::DohProxy;
use voindns_proto::UpstreamSel;

fn build_query(host: &str) -> Vec<u8> {
    // Header: ID=0x1234, RD set, QDCOUNT=1.
    let mut q = vec![0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    for label in host.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0); // root label
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // QTYPE=A, QCLASS=IN
    q
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_resolves_via_doh() {
    let addr: SocketAddr = "127.0.0.1:15353".parse().unwrap();
    let proxy = DohProxy::start(addr, &UpstreamSel::Cloudflare)
        .await
        .expect("start proxy");
    tokio::time::sleep(Duration::from_millis(150)).await;

    let query = build_query("example.com");
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.send_to(&query, addr).await.unwrap();

    let mut buf = vec![0u8; 1500];
    let n = match tokio::time::timeout(Duration::from_secs(8), sock.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => n,
        _ => {
            eprintln!("SKIP: no DoH response within timeout (network unavailable?)");
            let _ = proxy.stop().await;
            return;
        }
    };
    let resp = &buf[..n];

    assert!(n >= 12, "short DNS response ({n} bytes)");
    assert_eq!(&resp[0..2], &query[0..2], "response ID must echo the query");
    assert_eq!(resp[3] & 0x0f, 0, "RCODE must be NOERROR");
    let ancount = u16::from_be_bytes([resp[6], resp[7]]);
    assert!(ancount >= 1, "expected >=1 answer, got {ancount}");

    eprintln!("OK: proxy returned {ancount} answer(s) for example.com via DoH");
    proxy.stop().await.expect("stop proxy");
}
