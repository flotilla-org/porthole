use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::surface::SurfaceId;
use porthole_protocol::input::{PointerMoveRequest, PointerMoveResponse};

use crate::{routes::errors::ApiError, state::AppState};

pub async fn post_pointer_move(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PointerMoveRequest>,
) -> Result<Json<PointerMoveResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let units = req.units;
    let spec = (&req).into();
    state.input.pointer_move(&surface_id, &spec, units).await?;
    Ok(Json(PointerMoveResponse {
        surface_id: surface_id.to_string(),
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
        ErrorCode, PortholeError,
        display::{DisplayId, DisplayInfo, Rect},
        in_memory::InMemoryAdapter,
        memory_adapter::{MemoryAdapter, WindowSpec},
        surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState},
    };
    use porthole_protocol::{error::WireError, input::PointerMoveResponse};
    use tower::ServiceExt;

    use crate::{server::build_router, state::AppState};

    fn display_with_scale(scale: f64) -> DisplayInfo {
        DisplayInfo {
            id: DisplayId::new("d0"),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
            },
            scale,
            primary: true,
            focused: true,
        }
    }

    async fn router_with_window(outer: Rect, scale: f64) -> (axum::Router, SurfaceId, Arc<MemoryAdapter>) {
        let mut builder = MemoryAdapter::builder().display(display_with_scale(scale));
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

    async fn post(router: axum::Router, uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
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
    async fn post_pointer_move_returns_ok_and_updates_cursor_state() {
        // State-based assertion: after pointer_move, the adapter's cursor
        // position reflects the screen-global coords (window outer + spec).
        let (router, id, adapter) = router_with_window(
            Rect {
                x: 200.0,
                y: 100.0,
                w: 800.0,
                h: 600.0,
            },
            1.0,
        )
        .await;
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            serde_json::json!({ "x": 12.0, "y": 34.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let resp: PointerMoveResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.surface_id, id.to_string());
        let cursor = adapter.cursor();
        // Window outer (200, 100) + spec (12, 34) → cursor (212, 134).
        assert_eq!(cursor.x, 212.0);
        assert_eq!(cursor.y, 134.0);
    }

    #[tokio::test]
    async fn post_pointer_move_physical_units_scale_x_y() {
        // Display scale=2.0 set directly on the configured display — no
        // set_test_scale_for_snapshot workaround.
        let (router, id, adapter) = router_with_window(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 1200.0,
            },
            2.0,
        )
        .await;
        let (status, _) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            serde_json::json!({ "x": 1600.0, "y": 800.0, "units": "physical" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // Physical (1600, 800) ÷ 2 = logical (800, 400); window at (0,0) so
        // cursor lands at (800, 400) in screen-global.
        let cursor = adapter.cursor();
        assert_eq!(cursor.x, 800.0);
        assert_eq!(cursor.y, 400.0);
    }

    #[tokio::test]
    async fn post_pointer_move_returns_410_when_surface_dead() {
        let mut builder = MemoryAdapter::builder().display(display_with_scale(1.0));
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
        let router = build_router(state);
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            serde_json::json!({ "x": 0.0, "y": 0.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn post_pointer_move_surfaces_adapter_error() {
        // Error injection — keep on InMemoryAdapter (decorator pattern not
        // yet implemented; see #35).
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let router = build_router(state);
        adapter
            .set_next_pointer_move_result(Err(PortholeError::new(ErrorCode::InvalidCoordinate, "outside window")))
            .await;
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            serde_json::json!({ "x": 9999.0, "y": 9999.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::InvalidCoordinate);
    }
}
