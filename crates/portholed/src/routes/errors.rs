use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use porthole_core::launch::{ExistingSurfaceInfo, LaunchPipelineError};
use porthole_core::replace_pipeline::ReplacePipelineError;
use porthole_core::wait_pipeline::WaitPipelineError;
use porthole_core::{ErrorCode, PortholeError};
use porthole_protocol::error::{LaunchReturnedExistingBody, WireError};
use porthole_protocol::wait::WaitTimeoutBody;

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
                details: serde_json::to_value(WaitTimeoutBody {
                    elapsed_ms: info.elapsed_ms,
                    last_observed: info.last_observed,
                })
                .ok(),
            }),
        }
    }
}

impl From<LaunchPipelineError> for ApiError {
    fn from(err: LaunchPipelineError) -> Self {
        match err {
            LaunchPipelineError::Porthole(e) => Self(e.into()),
            LaunchPipelineError::ReturnedExisting(info) => Self(existing_to_wire(info)),
        }
    }
}

impl From<ReplacePipelineError> for ApiError {
    fn from(err: ReplacePipelineError) -> Self {
        match err {
            ReplacePipelineError::Porthole { error, old_handle_alive } => {
                let mut wire: WireError = error.into();
                let merged = match wire.details.take() {
                    Some(serde_json::Value::Object(mut map)) => {
                        map.insert(
                            "old_handle_alive".into(),
                            serde_json::Value::Bool(old_handle_alive),
                        );
                        serde_json::Value::Object(map)
                    }
                    _ => serde_json::json!({ "old_handle_alive": old_handle_alive }),
                };
                wire.details = Some(merged);
                Self(wire)
            }
            ReplacePipelineError::ReturnedExisting { info, old_handle_alive } => {
                let mut wire = existing_to_wire(info);
                if let Some(details) = wire.details.as_mut().and_then(|v| v.as_object_mut()) {
                    details.insert("old_handle_alive".into(), serde_json::Value::Bool(old_handle_alive));
                }
                Self(wire)
            }
        }
    }
}

fn existing_to_wire(info: ExistingSurfaceInfo) -> WireError {
    let body = LaunchReturnedExistingBody {
        ref_: info.ref_,
        app_name: info.app_name,
        title: info.title,
        pid: info.pid,
        cg_window_id: info.cg_window_id,
    };
    WireError {
        code: ErrorCode::LaunchReturnedExisting,
        message: "launch correlated to a preexisting surface (require_fresh_surface: true)".into(),
        details: serde_json::to_value(body).ok(),
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::SurfaceNotFound => StatusCode::NOT_FOUND,
            ErrorCode::SurfaceDead => StatusCode::GONE,
            ErrorCode::SystemPermissionNeeded => StatusCode::FORBIDDEN,
            ErrorCode::SystemPermissionRequestFailed => StatusCode::INTERNAL_SERVER_ERROR,
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
            ErrorCode::CloseFailed => StatusCode::CONFLICT,
            ErrorCode::LaunchReturnedExisting => StatusCode::CONFLICT,
        };
        (status, Json(self.0)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use porthole_core::wait::LastObserved;
    use porthole_core::wait_pipeline::{WaitPipelineError, WaitTimeoutInfo};

    use super::*;

    #[test]
    fn wait_timeout_api_error_includes_details() {
        let info = WaitTimeoutInfo {
            last_observed: LastObserved::Presence { alive: false },
            elapsed_ms: 5000,
        };
        let api_err = ApiError::from(WaitPipelineError::Timeout(info));
        assert_eq!(api_err.0.code, ErrorCode::WaitTimeout);
        let details = api_err.0.details.expect("details should be Some for Timeout");
        assert_eq!(details["elapsed_ms"], 5000);
        // LastObserved is internally-tagged: {"kind":"presence","alive":false}
        assert_eq!(details["last_observed"]["kind"], "presence");
        assert_eq!(details["last_observed"]["alive"], false);
    }

    #[test]
    fn porthole_error_api_error_has_no_details() {
        let err = PortholeError::new(ErrorCode::SurfaceDead, "gone");
        let api_err = ApiError::from(err);
        assert!(api_err.0.details.is_none());
    }

    #[test]
    fn porthole_error_with_details_propagates_to_wire() {
        let err = PortholeError::new(ErrorCode::SystemPermissionNeeded, "ax needed")
            .with_details(serde_json::json!({ "permission": "accessibility" }));
        let wire: porthole_protocol::error::WireError = err.into();
        assert_eq!(wire.code, ErrorCode::SystemPermissionNeeded);
        let details = wire.details.expect("details propagated");
        assert_eq!(details["permission"], "accessibility");
    }

    #[test]
    fn wire_error_details_skipped_when_none() {
        let w = WireError { code: ErrorCode::SurfaceDead, message: "gone".into(), details: None };
        let json = serde_json::to_string(&w).unwrap();
        assert!(!json.contains("details"), "details should be omitted when None: {json}");
    }

    #[test]
    fn close_failed_maps_to_409() {
        let err = PortholeError::new(ErrorCode::CloseFailed, "vetoed");
        let api_err = ApiError::from(err);
        let response = api_err.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn replace_porthole_merges_old_handle_alive_into_existing_details() {
        use porthole_core::replace_pipeline::ReplacePipelineError;
        let wrapped = PortholeError::new(ErrorCode::SystemPermissionNeeded, "ax")
            .with_details(serde_json::json!({
                "permission": "accessibility",
                "remediation": { "cli_command": "porthole onboard" }
            }));
        let api_err = ApiError::from(ReplacePipelineError::Porthole {
            error: wrapped,
            old_handle_alive: true,
        });
        let details = api_err.0.details.expect("details merged");
        assert_eq!(details["old_handle_alive"], true);
        assert_eq!(details["permission"], "accessibility");
        assert_eq!(details["remediation"]["cli_command"], "porthole onboard");
    }

    #[test]
    fn replace_porthole_populates_details_when_wrapped_has_none() {
        use porthole_core::replace_pipeline::ReplacePipelineError;
        let wrapped = PortholeError::new(ErrorCode::SurfaceDead, "gone");
        let api_err = ApiError::from(ReplacePipelineError::Porthole {
            error: wrapped,
            old_handle_alive: false,
        });
        let details = api_err.0.details.expect("details populated");
        assert_eq!(details["old_handle_alive"], false);
    }
}
