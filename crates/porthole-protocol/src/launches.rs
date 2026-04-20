use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchRequest {
    pub kind: LaunchKind,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default = "default_require_confidence")]
    pub require_confidence: WireConfidence,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_require_confidence() -> WireConfidence {
    WireConfidence::Strong
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaunchKind {
    Process(ProcessLaunch),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessLaunch {
    pub app: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireConfidence {
    Strong,
    Plausible,
    Weak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireCorrelation {
    Tag,
    PidTree,
    Temporal,
    DocumentMatch,
    FrontmostChanged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaunchResponse {
    pub launch_id: String,
    pub surface_id: SurfaceId,
    pub surface_was_preexisting: bool,
    pub confidence: WireConfidence,
    pub correlation: WireCorrelation,
}
