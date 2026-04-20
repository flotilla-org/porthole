use std::path::PathBuf;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use porthole_core::adapter::Adapter;
use tokio::net::UnixListener;
use tracing::info;

use crate::routes::{info as info_route, launches as launches_route, screenshot as screenshot_route};
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info_route::get_info))
        .route("/launches", post(launches_route::post_launches))
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
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
    use tower::ServiceExt;

    use super::*;

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
}
