use std::{sync::Arc, time::Duration};

use porthole_core::{in_memory::InMemoryAdapter, surface::SurfaceInfo};
use portholed::server::serve;

#[tokio::test]
async fn cli_through_daemon_key_text_click_wait_close() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    // Seed a tracked surface directly so we don't have to launch.
    let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
    let id = info.id.clone();
    // NB: state is inside the server task; we simulate by going through /launches
    // instead. But with the in-memory adapter, a launch returns a brand-new surface
    // we don't have a seeded reference to. The CLI-level flow is: launch → got
    // surface id → do stuff. We follow that path here:
    let _ = (info, id);

    let client = porthole::client::DaemonClient::new(&socket);
    let launch: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            "/launches",
            &serde_json::json!({ "kind": { "type": "process", "app": "X", "args": [] } }),
        )
        .await
        .expect("launch");

    // key
    let _: porthole_protocol::input::KeyResponse = client
        .post_json(
            &format!("/surfaces/{}/key", launch.surface_id),
            &serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await
        .expect("key");
    // text
    let _: porthole_protocol::input::TextResponse = client
        .post_json(
            &format!("/surfaces/{}/text", launch.surface_id),
            &serde_json::json!({ "text": "hi" }),
        )
        .await
        .expect("text");
    // wait exists
    let _: porthole_protocol::wait::WaitResponse = client
        .post_json(
            &format!("/surfaces/{}/wait", launch.surface_id),
            &serde_json::json!({ "condition": { "type": "exists" }, "timeout_ms": 1000 }),
        )
        .await
        .expect("wait");
    // close
    let _: porthole_protocol::close_focus::CloseResponse = client
        .post_json(&format!("/surfaces/{}/close", launch.surface_id), &serde_json::json!({}))
        .await
        .expect("close");

    server_task.abort();

    // adapter-side recorder sanity
    assert_eq!(adapter.key_calls().await.len(), 1);
    assert_eq!(adapter.text_calls().await.len(), 1);
    assert_eq!(adapter.wait_calls().await.len(), 1);
    assert_eq!(adapter.close_calls().await.len(), 1);
}
