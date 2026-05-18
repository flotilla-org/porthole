use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use porthole_core::{ErrorCode, PortholeError, agent_policy::ActionClass};
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};

use crate::{
    capture_registry::CaptureRegistryError,
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_synthetic(State(state): State<AppState>) -> Result<Json<CreateCaptureSessionResponse>, ApiError> {
    state.capture.create_synthetic_session().map(Json).map_err(capture_error_to_api)
}

pub async fn post_surface(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<CreateCaptureSessionResponse>, ApiError> {
    let surface_id = porthole_core::surface::SurfaceId::from(id);
    let execution = authorize_surface_actions(
        &state,
        &headers,
        surface_id.as_str(),
        &[ActionClass::Observe, ActionClass::Record],
        Some("record surface"),
    )
    .await?;
    let surface = state.handles.require_alive(&surface_id).await?;
    let response = state
        .capture
        .create_surface_session(state.adapter.clone(), surface, execution.agent_id.clone())
        .await
        .map_err(capture_error_to_api)?;
    let audit_state = state.clone();
    // Do not delay the initial frame handoff on audit persistence; capture
    // startup can race tight fd consumers in tests and real clients.
    tokio::spawn(async move {
        if let Err(error) = complete_route_execution(&audit_state, execution, "/capture-sessions/surfaces/{id}").await {
            tracing::warn!(
                code = %error.0.code,
                message = %error.0.message,
                "failed to write surface capture-session route execution audit"
            );
        }
    });
    Ok(Json(response))
}

pub async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<CaptureSessionResponse>, ApiError> {
    state.capture.get_session(&id).map(Json).map_err(capture_error_to_api)
}

pub async fn delete_session(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    state
        .capture
        .close_session(&id)
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(capture_error_to_api)
}

fn capture_error_to_api(error: CaptureRegistryError) -> ApiError {
    let code = match error {
        CaptureRegistryError::UnknownSession(_) => ErrorCode::SurfaceNotFound,
        CaptureRegistryError::Porthole(error) => return ApiError(error.into()),
        CaptureRegistryError::Poisoned | CaptureRegistryError::Io(_) => ErrorCode::InternalError,
        CaptureRegistryError::Failed { .. } => ErrorCode::InternalError,
        CaptureRegistryError::NotReady { .. } | CaptureRegistryError::Closed { .. } => ErrorCode::InvalidArgument,
        CaptureRegistryError::FdSocketDisabled | CaptureRegistryError::Capture(_) => ErrorCode::InvalidArgument,
    };
    ApiError(PortholeError::new(code, error.to_string()).into())
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;

    use super::*;

    #[test]
    fn failed_capture_session_maps_to_internal_error() {
        let response = capture_error_to_api(CaptureRegistryError::Failed {
            session_id: "capture-1".to_string(),
            message: "producer stopped".to_string(),
        })
        .into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn closed_capture_session_maps_to_invalid_argument() {
        let response = capture_error_to_api(CaptureRegistryError::Closed {
            session_id: "capture-1".to_string(),
            message: "capture stream ended".to_string(),
        })
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
