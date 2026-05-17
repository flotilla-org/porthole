use axum::{
    Json,
    extract::{Path, Query, State},
};
use porthole_core::surface::SurfaceId;
use porthole_protocol::content_rect::{ContentRectQuery, ContentRectResponse};

use crate::{routes::errors::ApiError, state::AppState};

pub async fn get_content_rect(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ContentRectQuery>,
) -> Result<Json<ContentRectResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let info = state.input.content_rect(&surface_id, q.units).await?;
    Ok(Json(ContentRectResponse {
        x: info.rect.x,
        y: info.rect.y,
        w: info.rect.w,
        h: info.rect.h,
        units: q.units,
        role: info.role,
        descent: info.descent,
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
        content_rect::Descent,
        display::{DisplayId, DisplayInfo, Rect},
        in_memory::InMemoryAdapter,
        input::CoordUnits,
        memory_adapter::{MemoryAdapter, WindowSpec},
        surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState},
    };
    use porthole_protocol::{content_rect::ContentRectResponse, error::WireError};
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

    async fn router_with_window(
        builder_setup: impl FnOnce(&mut porthole_core::memory_adapter::MemoryAdapterBuilder) -> SurfaceId,
        scale: f64,
    ) -> (axum::Router, SurfaceId, Arc<MemoryAdapter>) {
        let mut builder = MemoryAdapter::builder().display(display_with_scale(scale));
        let id = builder_setup(&mut builder);
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

    async fn get(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder().method(Method::GET).uri(uri).body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn get_content_rect_returns_ok_with_default_units() {
        // Window 800x600; default title_bar_h=28 → content_rect (0,28,800,572).
        let (router, id, _) = router_with_window(
            |b| {
                b.window(WindowSpec::new(
                    4242,
                    1,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 800.0,
                        h: 600.0,
                    },
                ))
            },
            1.0,
        )
        .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect")).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.x, 0.0);
        assert_eq!(resp.y, 28.0);
        assert_eq!(resp.w, 800.0);
        assert_eq!(resp.h, 572.0);
        assert!(matches!(resp.units, CoordUnits::Logical));
        assert_eq!(resp.role, "AXScrollArea");
        assert!(matches!(resp.descent, Descent::Contents));
    }

    #[tokio::test]
    async fn get_content_rect_with_physical_units_scales_rect() {
        // Display scale=2.0 lives directly on the configured display — no
        // set_test_scale_for_snapshot workaround needed.
        let (router, id, _) = router_with_window(
            |b| {
                b.window(WindowSpec::new(
                    4242,
                    1,
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 800.0,
                        h: 600.0,
                    },
                ))
            },
            2.0,
        )
        .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect?units=physical")).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        // Logical content (0,28,800,572) × 2 = (0,56,1600,1144).
        assert_eq!(resp.x, 0.0);
        assert_eq!(resp.y, 56.0);
        assert_eq!(resp.w, 1600.0);
        assert_eq!(resp.h, 1144.0);
        assert!(matches!(resp.units, CoordUnits::Physical));
    }

    #[tokio::test]
    async fn get_content_rect_passes_descent_and_role_through() {
        // Per-window overrides on WindowSpec replace the old set_next_content_rect
        // scripting.
        let (router, id, _) = router_with_window(
            |b| {
                b.window(
                    WindowSpec::new(
                        4242,
                        1,
                        Rect {
                            x: 0.0,
                            y: 0.0,
                            w: 100.0,
                            h: 220.0,
                        },
                    )
                    .with_role("AXGroup")
                    .with_descent(Descent::LargestChild)
                    .with_content_rect_override(Rect {
                        x: 10.0,
                        y: 20.0,
                        w: 100.0,
                        h: 200.0,
                    }),
                )
            },
            1.0,
        )
        .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect")).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.role, "AXGroup");
        assert!(matches!(resp.descent, Descent::LargestChild));
        assert_eq!(resp.x, 10.0);
        assert_eq!(resp.y, 20.0);
    }

    #[tokio::test]
    async fn get_content_rect_returns_410_when_surface_dead() {
        // The dead-handle path lives in HandleStore, not the adapter — same on
        // either fake. Using MemoryAdapter here keeps the file uniform.
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
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect")).await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn get_content_rect_returns_422_when_unavailable() {
        // Error injection is the documented escape hatch — keep this case on
        // InMemoryAdapter until a decorator wrapper lands. See #35.
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let router = build_router(state);
        adapter
            .set_next_content_rect(Err(PortholeError::new(
                ErrorCode::ContentRectUnavailable,
                "no usable content child",
            )))
            .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::ContentRectUnavailable);
    }
}
