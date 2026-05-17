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
    state.input.place(&surface_id, req.rect, req.units).await?;
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
        memory_adapter::{MemoryAdapter, WindowSpec},
        surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState},
    };
    use porthole_protocol::{error::WireError, placement::PlaceResponse};
    use tower::ServiceExt;

    use crate::{server::build_router, state::AppState};

    /// Build a router whose adapter is a `MemoryAdapter` configured with one
    /// window. The window is also inserted into the daemon's `HandleStore` so
    /// route paths that look up the surface by id see it as alive.
    async fn router_with_alive_window(outer: Rect) -> (axum::Router, SurfaceId, Arc<MemoryAdapter>) {
        let mut builder = MemoryAdapter::builder();
        let id = builder.window(WindowSpec::new(4242, 1, outer));
        let adapter = Arc::new(builder.build());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo {
            id: id.clone(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: None,
            app_name: None,
            pid: Some(4242),
            parent_surface_id: None,
            cg_window_id: Some(1),
        };
        state.handles.insert(info).await;
        (build_router(state), id, adapter)
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
    async fn post_place_returns_ok_and_updates_window_state() {
        // State-based assertion: place writes the new outer_rect onto the
        // window; we read it back from the adapter rather than checking a
        // record of calls.
        let (router, id, adapter) = router_with_alive_window(Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        })
        .await;
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
        let snap = adapter.window(&id).expect("window in state");
        assert_eq!(
            snap.outer_rect,
            Rect {
                x: 10.0,
                y: 20.0,
                w: 800.0,
                h: 600.0,
            }
        );
    }

    #[tokio::test]
    async fn post_place_rejects_non_positive_size() {
        let (router, id, _) = router_with_alive_window(Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        })
        .await;
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

    /// Build a router whose surface is tracked in the HandleStore *but marked
    /// dead* — exercises the SurfaceDead path before the request ever reaches
    /// the adapter.
    async fn router_with_dead_handle() -> (axum::Router, SurfaceId) {
        let mut builder = MemoryAdapter::builder();
        let id = builder.window(WindowSpec::new(
            4242,
            1,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            },
        ));
        let adapter = Arc::new(builder.build());
        let state = AppState::new(adapter);
        let info = SurfaceInfo {
            id: id.clone(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: None,
            app_name: None,
            pid: Some(4242),
            parent_surface_id: None,
            cg_window_id: Some(1),
        };
        state.handles.insert(info).await;
        state.handles.mark_dead(&id).await.unwrap();
        (build_router(state), id)
    }

    #[tokio::test]
    async fn post_place_returns_surface_dead_when_handle_marked_dead() {
        let (router, id) = router_with_dead_handle().await;
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
