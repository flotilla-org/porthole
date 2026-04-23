use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use porthole_core::adapter::Adapter;
use tokio::net::UnixListener;
use tracing::info;

use crate::routes::{
    attach as attach_route,
    attention as attention_route,
    close_focus as close_focus_route,
    info as info_route,
    input as input_route,
    launches as launches_route,
    replace as replace_route,
    screenshot as screenshot_route,
    wait as wait_route,
};
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info_route::get_info))
        .route("/attention", get(attention_route::get_attention))
        .route("/displays", get(attention_route::get_displays))
        .route("/launches", post(launches_route::post_launches))
        .route("/surfaces/search", post(attach_route::post_search))
        .route("/surfaces/track", post(attach_route::post_track))
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
        .route("/surfaces/{id}/key", post(input_route::post_key))
        .route("/surfaces/{id}/text", post(input_route::post_text))
        .route("/surfaces/{id}/click", post(input_route::post_click))
        .route("/surfaces/{id}/scroll", post(input_route::post_scroll))
        .route("/surfaces/{id}/wait", post(wait_route::post_wait))
        .route("/surfaces/{id}/replace", post(replace_route::post_replace))
        .route("/surfaces/{id}/close", post(close_focus_route::post_close))
        .route("/surfaces/{id}/focus", post(close_focus_route::post_focus))
        .with_state(state)
}

pub async fn serve(adapter: Arc<dyn Adapter>, socket_path: PathBuf) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let listener = UnixListener::bind(&socket_path)?;
    info!(socket = %socket_path.display(), "portholed listening");
    let app = build_router(AppState::new(adapter));
    axum::serve(listener, app).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use porthole_core::in_memory::InMemoryAdapter;
    use porthole_core::surface::SurfaceInfo;
    use tower::ServiceExt;

    use super::*;

    async fn router_with_tracked_surface() -> (Router, String) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
        let id = info.id.to_string();
        state.handles.insert(info).await;
        (build_router(state), id)
    }

    async fn post(router: Router, uri: &str, body: serde_json::Value) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn get_info_returns_adapter_info() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(info.adapters.len(), 1);
        assert_eq!(info.adapters[0].name, "in-memory");
    }

    #[tokio::test]
    async fn post_launch_then_screenshot_roundtrips() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let launch_body = serde_json::json!({
            "kind": { "type": "process", "app": "test", "args": [] },
            "require_confidence": "strong"
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri("/launches")
            .header("content-type", "application/json")
            .body(Body::from(launch_body.to_string()))
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let launch: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();

        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/surfaces/{}/screenshot", launch.surface_id))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
        let shot: porthole_protocol::screenshot::ScreenshotResponse = serde_json::from_slice(&body).unwrap();
        assert!(!shot.png_base64.is_empty());
    }

    #[tokio::test]
    async fn post_key_sends_events() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(
            router,
            &format!("/surfaces/{id}/key"),
            serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::KeyResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.events_sent, 1);
    }

    #[tokio::test]
    async fn post_key_with_unsupported_name_returns_bad_request() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(
            router,
            &format!("/surfaces/{id}/key"),
            serde_json::json!({ "events": [{ "key": "NotAKey" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_text_reports_char_count() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(router, &format!("/surfaces/{id}/text"), serde_json::json!({ "text": "hi" })).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::TextResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.chars_sent, 2);
    }

    #[tokio::test]
    async fn post_close_marks_surface_dead() {
        let (router, id) = router_with_tracked_surface().await;
        let res = post(router.clone(), &format!("/surfaces/{id}/close"), serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        // Subsequent operations should 410 (GONE)
        let res = post(router, &format!("/surfaces/{id}/focus"), serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn get_attention_returns_default() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/attention").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_displays_returns_list() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/displays").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::attention::DisplaysResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.displays.len(), 1);
    }

    #[tokio::test]
    async fn post_search_returns_candidates_from_adapter_script() {
        use porthole_core::search::Candidate;
        let adapter = Arc::new(InMemoryAdapter::new());
        let candidate = Candidate {
            ref_: "ref_abc".into(),
            app_name: Some("X".into()),
            title: Some("t".into()),
            pid: 1,
            cg_window_id: 7,
        };
        adapter.set_next_search_result(Ok(vec![candidate])).await;
        let router = build_router(AppState::new(adapter));
        let res = post(router, "/surfaces/search", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::search::SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.candidates.len(), 1);
    }

    #[tokio::test]
    async fn post_search_with_empty_adapter_result_returns_empty_candidates_list() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_next_search_result(Ok(vec![])).await;
        let router = build_router(AppState::new(adapter));
        let res = post(router, "/surfaces/search", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::search::SearchResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.candidates.is_empty());
    }

    #[tokio::test]
    async fn post_track_mints_handle_and_idempotent_reuse() {
        use porthole_core::search::encode_ref;
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter.clone()));

        let r = encode_ref(1, 7);
        let body = serde_json::json!({ "ref": r });

        // First call: script window_alive to return an alive surface.
        let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
        info.cg_window_id = Some(7);
        info.app_name = Some("X".into());
        adapter.set_next_window_alive_result(Ok(Some(info))).await;
        let res = post(router.clone(), "/surfaces/track", body.clone()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let first_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let first: porthole_protocol::search::TrackResponse = serde_json::from_slice(&first_body).unwrap();
        assert!(!first.reused_existing_handle);

        // Second call: script another alive surface with same cg_window_id.
        // track_or_get should find the existing handle and return reused=true.
        let mut info2 = SurfaceInfo::window(SurfaceId::new(), 1);
        info2.cg_window_id = Some(7);
        info2.app_name = Some("X".into());
        adapter.set_next_window_alive_result(Ok(Some(info2))).await;
        let res = post(router, "/surfaces/track", body).await;
        assert_eq!(res.status(), StatusCode::OK);
        let second_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let second: porthole_protocol::search::TrackResponse = serde_json::from_slice(&second_body).unwrap();
        assert!(second.reused_existing_handle);
        assert_eq!(second.surface_id, first.surface_id);
    }

    #[tokio::test]
    async fn post_track_with_malformed_ref_returns_not_found() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let res = post(router, "/surfaces/track", serde_json::json!({ "ref": "junk" })).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_replace_inherits_snapshot_when_no_placement() {
        use porthole_core::display::DisplayId;
        use porthole_core::display::Rect;
        use porthole_core::placement::GeometrySnapshot;
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Seed an alive handle with cg_window_id.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(50);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect { x: 10.0, y: 20.0, w: 500.0, h: 400.0 },
            }))
            .await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::Applied);
    }

    #[tokio::test]
    async fn post_replace_with_empty_placement_does_not_inherit() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(51);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {}
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::NotRequested);
    }

    #[tokio::test]
    async fn post_launches_rejects_url_artifact() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let res = post(
            router,
            "/launches",
            serde_json::json!({
                "kind": { "type": "artifact", "path": "https://example.com" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn info_lists_slice_c_capabilities() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        let caps = &info.adapters[0].capabilities;
        for expected in &["launch_artifact", "placement", "replace", "auto_dismiss"] {
            assert!(caps.contains(&expected.to_string()), "missing capability: {expected}");
        }
    }

    #[tokio::test]
    async fn post_replace_close_failure_returns_409_with_old_handle_alive_body() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Seed an alive handle.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(50);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        // Script close to fail.
        adapter
            .set_next_close_result(Err(porthole_core::PortholeError::new(
                porthole_core::ErrorCode::CloseFailed,
                "save dialog blocking",
            )))
            .await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({ "kind": { "type": "artifact", "path": "/tmp/x.pdf" } }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        // Typed error code is preserved — still close_failed.
        assert_eq!(err.code, porthole_core::ErrorCode::CloseFailed);
        let details = err.details.expect("details populated");
        assert_eq!(details.get("old_handle_alive").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn post_replace_preserves_permission_needed_from_close() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(50);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        adapter
            .set_next_close_result(Err(porthole_core::PortholeError::new(
                porthole_core::ErrorCode::PermissionNeeded,
                "AX denied",
            )))
            .await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({ "kind": { "type": "artifact", "path": "/tmp/x.pdf" } }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::PermissionNeeded);
        let details = err.details.expect("details populated");
        assert_eq!(
            details.get("old_handle_alive").and_then(|v| v.as_bool()),
            Some(true),
            "permission_needed on close means surface is likely still alive"
        );
    }

    #[tokio::test]
    async fn post_replace_launch_failure_after_close_returns_old_handle_alive_false() {
        use porthole_core::in_memory::InMemoryAdapter;
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Alive handle to replace.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.cg_window_id = Some(50);
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;

        // Close will succeed (default). Make the artifact launch return weak
        // confidence, which will fail the Strong requirement.
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(999);
        outcome.confidence = porthole_core::adapter::Confidence::Weak;
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let router = build_router(state);
        let res = post(
            router,
            &format!("/surfaces/{old_id}/replace"),
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_confidence": "strong"
            }),
        )
        .await;
        // Expect non-success: LaunchCorrelationAmbiguous → 409 CONFLICT
        assert!(!res.status().is_success());
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        // Regardless of exact code, details should include old_handle_alive: false
        let details = err.details.expect("details populated after post-close failure");
        assert_eq!(
            details.get("old_handle_alive").and_then(|v| v.as_bool()),
            Some(false),
            "post-close failure must report old_handle_alive: false"
        );
    }

    #[tokio::test]
    async fn post_launches_require_fresh_returns_409_with_ref_in_body() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(100);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(321);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let router = build_router(AppState::new(adapter));
        let res = post(
            router,
            "/launches",
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_fresh_surface": true
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::LaunchReturnedExisting);
        let details = err.details.expect("details populated");
        assert!(details.get("ref").is_some());
        assert_eq!(details.get("cg_window_id").and_then(|v| v.as_u64()), Some(321));
    }
}
