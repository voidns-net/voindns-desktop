//! IPC layer: framing, request/response, and the status subscription, exercised
//! over a real local socket. Does not trigger DNS redirect (no root needed).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use voidns_core::{ipc, Controller};
use voidns_proto::{Command, ConnState, Event};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ipc_ping_status_subscribe() {
    // Windows: a namespaced pipe name; elsewhere: a filesystem socket path.
    let sock = if cfg!(windows) {
        format!("voidns-test-{}", std::process::id())
    } else {
        format!("/tmp/voidns-test-{}.sock", std::process::id())
    };
    std::env::set_var("VOIDNS_SOCK", &sock);

    let controller = Arc::new(Mutex::new(Controller::new(15354).expect("controller")));
    let server = tokio::spawn(ipc::serve(controller.clone()));
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Ping -> Pong
    match ipc::one_shot(&Command::Ping).await.expect("ping") {
        Event::Pong => {}
        other => panic!("expected Pong, got {other:?}"),
    }

    // GetStatus -> Disconnected
    match ipc::one_shot(&Command::GetStatus).await.expect("status") {
        Event::Status(s) => assert_eq!(s.state, ConnState::Disconnected),
        other => panic!("expected Status, got {other:?}"),
    }

    // Subscribe -> immediate current status
    let mut sub = ipc::Subscription::open().await.expect("subscribe");
    let first = tokio::time::timeout(Duration::from_secs(2), sub.next())
        .await
        .expect("subscribe timeout")
        .expect("subscribe recv")
        .expect("subscribe some");
    assert_eq!(first.state, ConnState::Disconnected);

    eprintln!("OK: IPC ping/status/subscribe roundtrip");
    server.abort();
    let _ = std::fs::remove_file(&sock);
}
