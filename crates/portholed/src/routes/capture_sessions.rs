use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};

use crate::{capture_registry::CaptureRegistryError, routes::errors::ApiError, state::AppState};

pub async fn post_synthetic(State(state): State<AppState>) -> Result<Json<CreateCaptureSessionResponse>, ApiError> {
    state.capture.create_synthetic_session().map(Json).map_err(capture_error_to_api)
}

pub async fn post_surface(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<CreateCaptureSessionResponse>, ApiError> {
    let surface_id = porthole_core::surface::SurfaceId::from(id);
    let surface = state.handles.require_alive(&surface_id).await?;
    state
        .capture
        .create_surface_session(state.adapter.clone(), surface)
        .await
        .map(Json)
        .map_err(capture_error_to_api)
}

pub async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<CaptureSessionResponse>, ApiError> {
    state.capture.get_session(&id).map(Json).map_err(capture_error_to_api)
}

fn capture_error_to_api(error: CaptureRegistryError) -> ApiError {
    let code = match error {
        CaptureRegistryError::UnknownSession(_) => ErrorCode::SurfaceNotFound,
        CaptureRegistryError::Porthole(error) => return ApiError(error.into()),
        CaptureRegistryError::Poisoned | CaptureRegistryError::Io(_) => ErrorCode::InternalError,
        CaptureRegistryError::FdSocketDisabled | CaptureRegistryError::Capture(_) => ErrorCode::InvalidArgument,
    };
    ApiError(PortholeError::new(code, error.to_string()).into())
}
