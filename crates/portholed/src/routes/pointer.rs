use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::input::{PointerMoveRequest, PointerMoveResponse};

use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution, with_route},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_pointer_move(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<PointerMoveRequest>,
) -> Result<Json<PointerMoveResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Drive], Some("move pointer")).await?;
    let units = req.units;
    let spec = (&req).into();
    state.input.pointer_move(&surface_id, &spec, units).await?;
    complete_route_execution(&state, with_route(execution, "/surfaces/{id}/pointer/move")).await?;
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
        agent_policy::{ActionClass, DurationSpec, TargetSelector},
        in_memory::InMemoryAdapter,
        surface::{SurfaceId, SurfaceInfo},
    };
    use porthole_protocol::{error::WireError, input::PointerMoveResponse};
    use tower::ServiceExt;

    use crate::{agent_store::AgentPolicyStore, server::build_router, state::AppState};

    async fn router_with_alive_surface() -> (axum::Router, SurfaceId, Arc<InMemoryAdapter>, String) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let identity = store.create_identity("agent", None, 1_000).await.unwrap();
        let state = AppState::new_with_agent_policy(adapter.clone(), store.clone(), crate::events::EventBus::new());
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let request = store
            .create_pending_request(
                identity.agent_id.clone(),
                TargetSelector::Surface { surface_id: id.clone() },
                vec![ActionClass::Drive],
                None,
                1_001,
            )
            .await
            .unwrap();
        store
            .approve_request(&request.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), 1_002)
            .await
            .unwrap();
        let router = build_router(state);
        (router, id, adapter, identity.token)
    }

    async fn post(router: axum::Router, uri: &str, token: Option<&str>, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let req = builder.body(Body::from(body.to_string())).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({}));
        (status, json)
    }

    #[tokio::test]
    async fn post_pointer_move_returns_ok_and_records_adapter_call() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            Some(&token),
            serde_json::json!({ "x": 12.0, "y": 34.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let resp: PointerMoveResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.surface_id, id.to_string());
        let calls = adapter.pointer_move_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.x, 12.0);
        assert_eq!(calls[0].1.y, 34.0);
    }

    #[tokio::test]
    async fn post_pointer_move_physical_units_scale_x_y() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        adapter.set_test_scale_for_snapshot(2.0).await;
        let (status, _) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            Some(&token),
            serde_json::json!({ "x": 1600.0, "y": 800.0, "units": "physical" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let calls = adapter.pointer_move_calls().await;
        assert_eq!(calls[0].1.x, 800.0);
        assert_eq!(calls[0].1.y, 400.0);
    }

    #[tokio::test]
    async fn post_pointer_move_returns_410_when_surface_dead() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let identity = store.create_identity("agent", None, 1_000).await.unwrap();
        let state = AppState::new_with_agent_policy(adapter.clone(), store.clone(), crate::events::EventBus::new());
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let request = store
            .create_pending_request(
                identity.agent_id,
                TargetSelector::Surface { surface_id: id.clone() },
                vec![ActionClass::Drive],
                None,
                1_001,
            )
            .await
            .unwrap();
        store
            .approve_request(&request.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), 1_002)
            .await
            .unwrap();
        state.handles.mark_dead(&id).await.unwrap();
        let router = build_router(state);
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            Some(&identity.token),
            serde_json::json!({ "x": 0.0, "y": 0.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::GONE);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn post_pointer_move_surfaces_adapter_error() {
        let (router, id, adapter, token) = router_with_alive_surface().await;
        adapter
            .set_next_pointer_move_result(Err(PortholeError::new(ErrorCode::InvalidCoordinate, "outside window")))
            .await;
        let (status, body) = post(
            router,
            &format!("/surfaces/{id}/pointer/move"),
            Some(&token),
            serde_json::json!({ "x": 9999.0, "y": 9999.0 }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::InvalidCoordinate);
    }
}
