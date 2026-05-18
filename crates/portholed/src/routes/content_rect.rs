use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::content_rect::{ContentRectQuery, ContentRectResponse};

use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn get_content_rect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<ContentRectQuery>,
) -> Result<Json<ContentRectResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(
        &state,
        &headers,
        surface_id.as_str(),
        &[ActionClass::Observe],
        Some("read content rect"),
    )
    .await?;
    let info = state.input.content_rect(&surface_id, q.units).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/content-rect").await?;
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
        agent_policy::{ActionClass, DurationSpec, TargetSelector},
        content_rect::{ContentRectInfo, Descent},
        display::Rect,
        in_memory::InMemoryAdapter,
        input::CoordUnits,
        surface::{SurfaceId, SurfaceInfo},
    };
    use porthole_protocol::{content_rect::ContentRectResponse, error::WireError};
    use tower::ServiceExt;

    use crate::{server::build_router, state::AppState};

    async fn router_with_alive_surface() -> (axum::Router, SurfaceId, Arc<InMemoryAdapter>, String) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let token = authorize_surface(&state, &id).await;
        let router = build_router(state);
        (router, id, adapter, token)
    }

    async fn authorize_surface(state: &AppState, id: &SurfaceId) -> String {
        let identity = state.agent_store.create_identity("agent", None, 1_000).await.unwrap();
        let request = state
            .agent_store
            .create_pending_request(
                identity.agent_id,
                TargetSelector::Surface { surface_id: id.clone() },
                vec![ActionClass::Observe],
                None,
                1_001,
            )
            .await
            .unwrap();
        state
            .agent_store
            .approve_request(&request.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), 1_002)
            .await
            .unwrap();
        identity.token
    }

    async fn get(router: axum::Router, uri: &str, token: &str) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn get_content_rect_returns_ok_with_default_units() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect"), &token).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        // Default in-memory fake: x=0, y=28, w=800, h=572, role=AXScrollArea.
        assert_eq!(resp.x, 0.0);
        assert_eq!(resp.y, 28.0);
        assert_eq!(resp.w, 800.0);
        assert_eq!(resp.h, 572.0);
        assert!(matches!(resp.units, CoordUnits::Logical));
        assert_eq!(resp.role, "AXScrollArea");
        assert!(matches!(resp.descent, Descent::Contents));
        assert_eq!(adapter.content_rect_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn get_content_rect_with_physical_units_scales_rect() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        adapter.set_test_scale_for_snapshot(2.0).await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect?units=physical"), &token).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.x, 0.0);
        assert_eq!(resp.y, 56.0);
        assert_eq!(resp.w, 1600.0);
        assert_eq!(resp.h, 1144.0);
        assert!(matches!(resp.units, CoordUnits::Physical));
    }

    #[tokio::test]
    async fn get_content_rect_returns_410_when_surface_dead() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let token = authorize_surface(&state, &id).await;
        state.handles.mark_dead(&id).await.unwrap();
        let router = build_router(state);
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect"), &token).await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn get_content_rect_returns_422_when_unavailable() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        adapter
            .set_next_content_rect(Err(PortholeError::new(
                ErrorCode::ContentRectUnavailable,
                "no usable content child",
            )))
            .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect"), &token).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::ContentRectUnavailable);
    }

    #[tokio::test]
    async fn get_content_rect_passes_descent_and_role_through() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        adapter
            .set_next_content_rect(Ok(ContentRectInfo {
                rect: Rect {
                    x: 10.0,
                    y: 20.0,
                    w: 100.0,
                    h: 200.0,
                },
                role: "AXGroup".into(),
                descent: Descent::LargestChild,
            }))
            .await;
        let (status, body) = get(router, &format!("/surfaces/{id}/content-rect"), &token).await;
        assert_eq!(status, StatusCode::OK);
        let resp: ContentRectResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.role, "AXGroup");
        assert!(matches!(resp.descent, Descent::LargestChild));
    }
}
