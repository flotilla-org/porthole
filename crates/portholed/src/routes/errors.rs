use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::error::WireError;

pub struct ApiError(pub WireError);

impl From<PortholeError> for ApiError {
    fn from(err: PortholeError) -> Self {
        Self(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::SurfaceNotFound => StatusCode::NOT_FOUND,
            ErrorCode::SurfaceDead => StatusCode::GONE,
            ErrorCode::PermissionNeeded => StatusCode::FORBIDDEN,
            ErrorCode::LaunchCorrelationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::LaunchCorrelationAmbiguous => StatusCode::CONFLICT,
            ErrorCode::LaunchTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::CandidateRefUnknown => StatusCode::NOT_FOUND,
            ErrorCode::AdapterUnsupported => StatusCode::BAD_REQUEST,
            ErrorCode::CapabilityMissing => StatusCode::NOT_IMPLEMENTED,
        };
        (status, Json(self.0)).into_response()
    }
}
