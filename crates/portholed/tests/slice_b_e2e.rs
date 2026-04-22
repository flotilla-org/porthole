use std::sync::Arc;
use std::time::Duration;

use porthole_core::in_memory::InMemoryAdapter;
use porthole_core::search::{encode_ref, Candidate};
use porthole_core::surface::{SurfaceId, SurfaceInfo};
use portholed::server::serve;

#[tokio::test]
async fn search_track_roundtrip_over_uds() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    // Seed one candidate + one window_alive result.
    let r = encode_ref(77, 123);
    adapter
        .set_next_search_result(Ok(vec![Candidate {
            ref_: r.clone(),
            app_name: Some("ScriptedApp".into()),
            title: Some("one".into()),
            pid: 77,
            cg_window_id: 123,
        }]))
        .await;
    let mut info = SurfaceInfo::window(SurfaceId::new(), 77);
    info.cg_window_id = Some(123);
    info.app_name = Some("ScriptedApp".into());
    adapter.set_next_window_alive_result(Ok(Some(info))).await;

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

    let client = porthole::client::DaemonClient::new(&socket);

    let search: porthole_protocol::search::SearchResponse = client
        .post_json(
            "/surfaces/search",
            &serde_json::json!({ "app_name": "ScriptedApp" }),
        )
        .await
        .expect("search");
    assert_eq!(search.candidates.len(), 1);
    assert_eq!(search.candidates[0].ref_, r);

    let track: porthole_protocol::search::TrackResponse = client
        .post_json(
            "/surfaces/track",
            &serde_json::json!({ "ref": r }),
        )
        .await
        .expect("track");
    assert!(!track.reused_existing_handle);
    assert_eq!(track.cg_window_id, 123);

    server_task.abort();
}
