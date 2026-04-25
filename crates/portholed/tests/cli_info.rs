use std::{sync::Arc, time::Duration};

use porthole_core::in_memory::InMemoryAdapter;
use portholed::server::serve;

#[tokio::test]
async fn daemon_serves_info_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    let socket_for_serve = socket.clone();
    let server_task = tokio::spawn(async move { serve(adapter, socket_for_serve).await });

    // Wait for the socket to appear (up to ~2s).
    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let client = porthole::client::DaemonClient::new(&socket);
    let info: porthole_protocol::info::InfoResponse = client.get_json("/info").await.expect("get_json");
    assert_eq!(info.adapters.len(), 1);
    assert_eq!(info.adapters[0].name, "in-memory");

    server_task.abort();
}
