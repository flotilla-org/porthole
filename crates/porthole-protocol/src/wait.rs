use serde::{Deserialize, Serialize};

use porthole_core::wait::{LastObserved, WaitCondition};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitRequest {
    pub condition: WaitCondition,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub session: Option<String>,
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitResponse {
    pub surface_id: String,
    pub condition: String,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WaitTimeoutBody {
    pub elapsed_ms: u64,
    pub last_observed: LastObserved,
}
