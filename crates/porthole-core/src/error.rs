use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    SurfaceNotFound,
    SurfaceDead,
    SystemPermissionNeeded,
    SystemPermissionRequestFailed,
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
            Self::SystemPermissionNeeded => "system_permission_needed",
            Self::SystemPermissionRequestFailed => "system_permission_request_failed",
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
    pub details: Option<serde_json::Value>,
}

impl PortholeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
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
        assert_eq!(ErrorCode::SystemPermissionNeeded.to_string(), "system_permission_needed");
        assert_eq!(
            ErrorCode::SystemPermissionRequestFailed.to_string(),
            "system_permission_request_failed"
        );
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

    #[test]
    fn with_details_attaches_json_object() {
        let err = PortholeError::new(ErrorCode::SystemPermissionNeeded, "accessibility needed")
            .with_details(serde_json::json!({ "permission": "accessibility" }));
        assert_eq!(err.code, ErrorCode::SystemPermissionNeeded);
        let details = err.details.expect("details set");
        assert_eq!(details["permission"], "accessibility");
    }

    #[test]
    fn default_constructor_leaves_details_none() {
        let err = PortholeError::new(ErrorCode::SurfaceDead, "gone");
        assert!(err.details.is_none());
    }
}
