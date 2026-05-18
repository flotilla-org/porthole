use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::placement::{PlaceRequest, PlaceResponse};

use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_place(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PlaceRequest>,
) -> Result<Json<PlaceResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Manage], Some("place surface")).await?;
    state.input.place(&surface_id, req.rect, req.units).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/place").await?;
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
        agent_policy::{ActionClass, DurationSpec, TargetSelector},
        display::Rect,
        in_memory::InMemoryAdapter,
        surface::{SurfaceId, SurfaceInfo},
    };
    use porthole_protocol::{error::WireError, placement::PlaceResponse};
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
                vec![ActionClass::Manage],
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

    async fn post_json(router: axum::Router, uri: &str, token: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
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
        let (router, id, adapter, token) = router_with_alive_surface().await;
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            &token,
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
        let (router, id, _, token) = router_with_alive_surface().await;
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            &token,
            serde_json::json!({ "rect": { "x": 0.0, "y": 0.0, "w": 0.0, "h": 100.0 } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::InvalidArgument);
    }

    #[tokio::test]
    async fn post_place_returns_surface_dead_when_handle_marked_dead() {
        let (router, id, _, token) = {
            let adapter = Arc::new(InMemoryAdapter::new());
            let state = AppState::new(adapter.clone());
            let info = SurfaceInfo::window(SurfaceId::new(), 1);
            let id = info.id.clone();
            state.handles.insert(info).await;
            let token = authorize_surface(&state, &id).await;
            state.handles.mark_dead(&id).await.unwrap();
            let router = build_router(state);
            (router, id, adapter, token)
        };
        let (status, body) = post_json(
            router,
            &format!("/surfaces/{id}/place"),
            &token,
            serde_json::json!({ "rect": { "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0 } }),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::SurfaceDead);
    }
}
