use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateSyntheticCaptureSessionResponse};

use crate::{capture_registry::CaptureRegistryError, routes::errors::ApiError, state::AppState};

pub async fn post_synthetic(State(state): State<AppState>) -> Result<Json<CreateSyntheticCaptureSessionResponse>, ApiError> {
    state.capture.create_synthetic_session().map(Json).map_err(capture_error_to_api)
}

pub async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<CaptureSessionResponse>, ApiError> {
    state.capture.get_session(&id).map(Json).map_err(capture_error_to_api)
}

fn capture_error_to_api(error: CaptureRegistryError) -> ApiError {
    let code = match error {
        CaptureRegistryError::UnknownSession(_) => ErrorCode::SurfaceNotFound,
        CaptureRegistryError::FdSocketDisabled
        | CaptureRegistryError::Poisoned
        | CaptureRegistryError::Capture(_)
        | CaptureRegistryError::Io(_) => ErrorCode::InvalidArgument,
    };
    ApiError(PortholeError::new(code, error.to_string()).into())
}
