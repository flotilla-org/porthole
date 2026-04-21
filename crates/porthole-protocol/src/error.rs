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
        Self { code: err.code, message: err.message, details: None }
    }
}
