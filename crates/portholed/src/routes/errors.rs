use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use porthole_core::wait_pipeline::WaitPipelineError;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::error::WireError;
pub struct ApiError(pub WireError);

impl From<PortholeError> for ApiError {
    fn from(err: PortholeError) -> Self {
        Self(err.into())
    }
}

impl From<WaitPipelineError> for ApiError {
    fn from(err: WaitPipelineError) -> Self {
        match err {
            WaitPipelineError::Porthole(e) => Self(e.into()),
            WaitPipelineError::Timeout(info) => Self(WireError {
                code: ErrorCode::WaitTimeout,
                message: format!("wait condition not satisfied within timeout ({}ms elapsed)", info.elapsed_ms),
            }),
            // Note: the `timeout` diagnostics live in the wire body beside code+message;
            // we'd need a richer WireError shape to carry them. For this slice, we
            // include the elapsed_ms in the message and add structured diagnostics
            // via a later events slice.
        }
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
            ErrorCode::WaitTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::UnknownKey => StatusCode::BAD_REQUEST,
            ErrorCode::InvalidCoordinate => StatusCode::BAD_REQUEST,
            ErrorCode::InvalidArgument => StatusCode::BAD_REQUEST,
        };
        (status, Json(self.0)).into_response()
    }
}
