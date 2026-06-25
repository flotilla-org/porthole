use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use porthole_core::{ErrorCode, PortholeError, agent_policy::ActionClass};
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};
use serde::Deserialize;

/// Query for `POST /capture-sessions/surfaces/{id}`. `native=true` requests
/// the platform native handle path; the default is the CPU-shm fd-socket path.
#[derive(Debug, Default, Deserialize)]
pub struct CaptureKindQuery {
    #[serde(default)]
    pub native: bool,
}

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
    Query(kind): Query<CaptureKindQuery>,
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
    let response = create_session(&state, kind.native, surface, execution.agent_id.clone())
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

async fn create_session(
    state: &AppState,
    native: bool,
    surface: porthole_core::surface::SurfaceInfo,
    agent_id: porthole_core::agent_policy::AgentId,
) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
    if native {
        #[cfg(target_os = "macos")]
        {
            return state.capture.create_native_surface_session(surface, agent_id).await;
        }
        #[cfg(target_os = "linux")]
        {
            let Some(kwin_adapter) = state.kwin_adapter.clone() else {
                return Err(CaptureRegistryError::from_porthole(PortholeError::new(
                    ErrorCode::AdapterUnsupported,
                    "native Linux capture sessions require a KDE Wayland KWin adapter",
                )));
            };
            return state.capture.create_native_surface_session(kwin_adapter, surface, agent_id).await;
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (surface, agent_id);
            return Err(CaptureRegistryError::from_porthole(PortholeError::new(
                ErrorCode::AdapterUnsupported,
                "native capture sessions are unsupported on this platform",
            )));
        }
    }
    state.capture.create_surface_session(state.adapter.clone(), surface, agent_id).await
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
