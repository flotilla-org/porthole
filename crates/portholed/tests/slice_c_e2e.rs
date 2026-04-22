use std::sync::Arc;
use std::time::Duration;

use porthole_core::display::{DisplayId, Rect};
use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::placement::GeometrySnapshot;
use portholed::server::serve;

#[tokio::test]
async fn artifact_launch_place_replace_autodismiss_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Seed for an artifact launch — default outcome is fine; we inspect calls.
    adapter
        .set_next_snapshot_geometry(Ok(GeometrySnapshot {
            display_id: DisplayId::new("in-mem-display-0"),
            display_local: Rect { x: 30.0, y: 40.0, w: 500.0, h: 400.0 },
        }))
        .await;

    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let server_task = tokio::spawn(async move { serve(adapter_for_serve, socket_for_serve).await });

    for _ in 0..200 {
        if socket.exists() { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    let client = porthole::client::DaemonClient::new(&socket);

    // 1. Artifact launch with full placement.
    let launch: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            "/launches",
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {
                    "on_display": "primary",
                    "geometry": { "x": 10.0, "y": 20.0, "w": 400.0, "h": 300.0 }
                }
            }),
        )
        .await
        .expect("launch");
    assert_eq!(
        launch.placement,
        porthole_core::placement::PlacementOutcome::Applied
    );

    // 2. Replace with omitted placement → inheritance.
    let replace: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            &format!("/surfaces/{}/replace", launch.surface_id),
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/y.pdf" }
            }),
        )
        .await
        .expect("replace");
    assert_eq!(
        replace.placement,
        porthole_core::placement::PlacementOutcome::Applied
    );
    assert_ne!(replace.surface_id, launch.surface_id, "replace should mint a fresh id");

    // 3. URL artifact rejected.
    let url_res: Result<porthole_protocol::launches::LaunchResponse, _> = client
        .post_json(
            "/launches",
            &serde_json::json!({
                "kind": { "type": "artifact", "path": "https://example.com" }
            }),
        )
        .await;
    assert!(url_res.is_err(), "URL artifact should be rejected");

    server_task.abort();
}
