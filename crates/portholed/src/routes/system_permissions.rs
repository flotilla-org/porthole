use axum::extract::State;
use axum::Json;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::system_permission::SystemPermissionPromptOutcome;
use serde::Deserialize;

use crate::routes::errors::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RequestBody {
    pub name: String,
}

pub async fn post_request(
    State(state): State<AppState>,
    Json(body): Json<RequestBody>,
) -> Result<Json<SystemPermissionPromptOutcome>, ApiError> {
    // Capability check first: if the adapter doesn't advertise
    // system_permission_prompt, return CapabilityMissing (501) without
    // dispatching.
    let caps = state.adapter.capabilities();
    if !caps.contains(&"system_permission_prompt") {
        return Err(ApiError::from(PortholeError::new(
            ErrorCode::CapabilityMissing,
            "adapter does not support system permission prompts",
        )));
    }

    let core_outcome = state
        .adapter
        .request_system_permission_prompt(&body.name)
        .await?;
    Ok(Json(core_outcome.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Method, Request, StatusCode};
    use porthole_core::in_memory::InMemoryAdapter;
    use porthole_protocol::error::WireError;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::server::build_router;

    async fn post_json(uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
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
    async fn in_memory_adapter_returns_capability_missing() {
        let (status, body) = post_json(
            "/system-permissions/request",
            serde_json::json!({ "name": "accessibility" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::CapabilityMissing);
    }
}
