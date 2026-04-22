use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    SurfaceNotFound,
    SurfaceDead,
    PermissionNeeded,
    LaunchCorrelationFailed,
    LaunchCorrelationAmbiguous,
    LaunchTimeout,
    CandidateRefUnknown,
    AdapterUnsupported,
    CapabilityMissing,
    WaitTimeout,
    UnknownKey,
    InvalidCoordinate,
    InvalidArgument,
    CloseFailed,
    LaunchReturnedExisting,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::SurfaceNotFound => "surface_not_found",
            Self::SurfaceDead => "surface_dead",
            Self::PermissionNeeded => "permission_needed",
            Self::LaunchCorrelationFailed => "launch_correlation_failed",
            Self::LaunchCorrelationAmbiguous => "launch_correlation_ambiguous",
            Self::LaunchTimeout => "launch_timeout",
            Self::CandidateRefUnknown => "candidate_ref_unknown",
            Self::AdapterUnsupported => "adapter_unsupported",
            Self::CapabilityMissing => "capability_missing",
            Self::WaitTimeout => "wait_timeout",
            Self::UnknownKey => "unknown_key",
            Self::InvalidCoordinate => "invalid_coordinate",
            Self::InvalidArgument => "invalid_argument",
            Self::CloseFailed => "close_failed",
            Self::LaunchReturnedExisting => "launch_returned_existing",
        };
        f.write_str(s)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct PortholeError {
    pub code: ErrorCode,
    pub message: String,
}

impl PortholeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }

    pub fn surface_not_found(id: &str) -> Self {
        Self::new(ErrorCode::SurfaceNotFound, format!("no tracked surface with id {id}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_display_matches_wire_string() {
        assert_eq!(ErrorCode::SurfaceNotFound.to_string(), "surface_not_found");
        assert_eq!(ErrorCode::LaunchCorrelationAmbiguous.to_string(), "launch_correlation_ambiguous");
    }

    #[test]
    fn surface_not_found_helper_sets_code() {
        let err = PortholeError::surface_not_found("abc");
        assert_eq!(err.code, ErrorCode::SurfaceNotFound);
        assert!(err.message.contains("abc"));
    }

    #[test]
    fn new_error_codes_display_as_snake_case() {
        assert_eq!(ErrorCode::WaitTimeout.to_string(), "wait_timeout");
        assert_eq!(ErrorCode::UnknownKey.to_string(), "unknown_key");
        assert_eq!(ErrorCode::InvalidCoordinate.to_string(), "invalid_coordinate");
        assert_eq!(ErrorCode::InvalidArgument.to_string(), "invalid_argument");
        assert_eq!(ErrorCode::CloseFailed.to_string(), "close_failed");
    }

    #[test]
    fn launch_returned_existing_display_is_snake_case() {
        assert_eq!(ErrorCode::LaunchReturnedExisting.to_string(), "launch_returned_existing");
    }
}
