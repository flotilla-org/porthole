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
    use porthole_core::permission::SystemPermissionPromptOutcome as CoreOutcome;
    use porthole_protocol::error::WireError;
    use porthole_protocol::system_permission::SystemPermissionRequestFailedBody;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::server::build_router;

    async fn post_json_with(
        adapter: Arc<InMemoryAdapter>,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
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

    async fn post_json(uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        post_json_with(Arc::new(InMemoryAdapter::new()), uri, body).await
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

    #[tokio::test]
    async fn request_returns_scripted_outcome_when_capability_advertised() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_system_permission_prompt_capability(true);
        adapter
            .set_next_request_system_permission_prompt(Ok(CoreOutcome {
                permission: "accessibility".into(),
                granted_before: false,
                granted_after: true,
                prompt_triggered: true,
                requires_daemon_restart: true,
                notes: "restart the daemon".into(),
            }))
            .await;

        let (status, body) = post_json_with(
            adapter,
            "/system-permissions/request",
            serde_json::json!({ "name": "accessibility" }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let outcome: SystemPermissionPromptOutcome = serde_json::from_value(body).unwrap();
        assert_eq!(outcome.permission, "accessibility");
        assert!(outcome.granted_after);
        assert!(outcome.prompt_triggered);
        assert!(outcome.requires_daemon_restart);
        assert_eq!(outcome.notes, "restart the daemon");
    }

    #[tokio::test]
    async fn request_propagates_request_failed_remediation_details() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_system_permission_prompt_capability(true);
        let failed = SystemPermissionRequestFailedBody {
            permission: "accessibility".into(),
            reason: "process is not running inside a .app bundle".into(),
            settings_path: "System Settings → Privacy & Security → Accessibility".into(),
            binary_path: "/path/to/portholed".into(),
        };
        adapter
            .set_next_request_system_permission_prompt(Err(PortholeError::new(
                ErrorCode::SystemPermissionRequestFailed,
                "cannot open prompt for accessibility",
            )
            .with_details(serde_json::to_value(&failed).unwrap())))
            .await;

        let (status, body) = post_json_with(
            adapter,
            "/system-permissions/request",
            serde_json::json!({ "name": "accessibility" }),
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::SystemPermissionRequestFailed);
        let details = err.details.expect("details present");
        let parsed: SystemPermissionRequestFailedBody =
            serde_json::from_value(details).expect("details deserialise");
        assert_eq!(parsed, failed);
    }
}
