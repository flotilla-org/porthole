use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use porthole_core::adapter::Adapter;
use tokio::net::UnixListener;
use tracing::info;

use crate::routes::{
    attention as attention_route,
    close_focus as close_focus_route,
    info as info_route,
    input as input_route,
    launches as launches_route,
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
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
        .route("/surfaces/{id}/key", post(input_route::post_key))
        .route("/surfaces/{id}/text", post(input_route::post_text))
        .route("/surfaces/{id}/click", post(input_route::post_click))
        .route("/surfaces/{id}/scroll", post(input_route::post_scroll))
        .route("/surfaces/{id}/wait", post(wait_route::post_wait))
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
}
