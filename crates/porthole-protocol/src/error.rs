use porthole_core::ErrorCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
}

impl From<porthole_core::PortholeError> for WireError {
    fn from(err: porthole_core::PortholeError) -> Self {
        Self { code: err.code, message: err.message }
    }
}
