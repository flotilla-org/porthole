use std::collections::BTreeMap;

pub use porthole_core::{
    display::Rect as PlacementRect,
    placement::{Anchor, DisplayTarget, PlacementOutcome, PlacementSpec},
};
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
    // NEW:
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_dismiss_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_fresh_surface: bool,
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
    Artifact(ArtifactLaunch),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactLaunch {
    pub path: String,
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
    pub placement: PlacementOutcome,
}

// ReplaceRequest is structurally identical to LaunchRequest — the old
// surface id comes from the URL path parameter. Keep the alias for clarity
// in route handlers and OpenAPI generation later.
pub type ReplaceRequest = LaunchRequest;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_launch_kind_serializes_with_path() {
        let json = serde_json::json!({
            "type": "artifact",
            "path": "/tmp/x.pdf"
        });
        let k: LaunchKind = serde_json::from_value(json).unwrap();
        match k {
            LaunchKind::Artifact(a) => assert_eq!(a.path, "/tmp/x.pdf"),
            _ => panic!("expected artifact"),
        }
    }

    #[test]
    fn launch_request_placement_key_absent_deserializes_as_none() {
        let json = r#"{"kind":{"type":"process","app":"x"}}"#;
        let req: LaunchRequest = serde_json::from_str(json).unwrap();
        assert!(req.placement.is_none());
    }

    #[test]
    fn launch_request_placement_empty_object_deserializes_as_some_default() {
        let json = r#"{"kind":{"type":"process","app":"x"},"placement":{}}"#;
        let req: LaunchRequest = serde_json::from_str(json).unwrap();
        assert!(req.placement.is_some());
        assert!(req.placement.unwrap().is_effectively_empty());
    }

    #[test]
    fn placement_outcome_applied_serializes() {
        let o = PlacementOutcome::Applied;
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"type":"applied"}"#);
    }

    #[test]
    fn close_failed_body_carries_old_handle_alive() {
        let body = crate::error::CloseFailedBody { old_handle_alive: true };
        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(json, r#"{"old_handle_alive":true}"#);
    }
}
