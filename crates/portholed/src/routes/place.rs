use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::surface::SurfaceId;
use porthole_protocol::placement::{PlaceRequest, PlaceResponse};

use crate::{routes::errors::ApiError, state::AppState};

pub async fn post_place(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PlaceRequest>,
) -> Result<Json<PlaceResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    state.input.place(&surface_id, req.rect).await?;
    Ok(Json(PlaceResponse {
        surface_id: surface_id.to_string(),
        placed: true,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use porthole_core::{
        display::Rect,
        in_memory::InMemoryAdapter,
        surface::{SurfaceId, SurfaceInfo},
    };
    use porthole_protocol::{error::WireError, placement::PlaceResponse};
    use tower::ServiceExt;

    use crate::{server::build_router, state::AppState};

    async fn router_with_alive_surface() -> (axum::Router, SurfaceId, Arc<InMemoryAdapter>) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let router = build_router(state);
        (router, id, adapter)
    }

    async fn post_json(router: axum::Router, uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn post_place_returns_ok_and_records_adapter_call() {
        let (router, id, adapter) = router_with_alive_surface().await;
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            serde_json::json!({ "rect": { "x": 10.0, "y": 20.0, "w": 800.0, "h": 600.0 } }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let resp: PlaceResponse = serde_json::from_value(body).unwrap();
        assert!(resp.placed);
        assert_eq!(resp.surface_id, id.to_string());
        let calls = adapter.place_surface_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 800.0,
                h: 600.0
            }
        );
    }

    #[tokio::test]
    async fn post_place_rejects_non_positive_size() {
        let (router, id, _) = router_with_alive_surface().await;
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            serde_json::json!({ "rect": { "x": 0.0, "y": 0.0, "w": 0.0, "h": 100.0 } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn post_place_returns_surface_dead_when_handle_marked_dead() {
        let (router, id, _) = {
            let adapter = Arc::new(InMemoryAdapter::new());
            let state = AppState::new(adapter.clone());
            let info = SurfaceInfo::window(SurfaceId::new(), 1);
            let id = info.id.clone();
            state.handles.insert(info).await;
            state.handles.mark_dead(&id).await.unwrap();
            let router = build_router(state);
            (router, id, adapter)
        };
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            serde_json::json!({ "rect": { "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0 } }),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::SurfaceDead);
    }
}
