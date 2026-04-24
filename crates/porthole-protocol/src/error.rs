use porthole_core::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
    /// Optional structured diagnostics. Callers that do not understand a given
    /// `code` can ignore this field. Fully back-compatible — absent from
    /// existing error responses; older callers simply ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl From<porthole_core::PortholeError> for WireError {
    fn from(err: porthole_core::PortholeError) -> Self {
        Self { code: err.code, message: err.message, details: err.details }
    }
}

/// Structured body for `LaunchReturnedExisting` errors.
/// Serialised into `WireError::details` as a JSON value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchReturnedExistingBody {
    #[serde(rename = "ref")]
    pub ref_: String,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}

/// Structured body for close-failed errors.
/// Serialised into `WireError::details` as a JSON value.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseFailedBody {
    pub old_handle_alive: bool,
}
