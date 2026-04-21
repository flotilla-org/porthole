use std::time::Duration;

use async_trait::async_trait;

use crate::attention::AttentionInfo;
use crate::display::DisplayInfo;
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::permission::PermissionStatus;
use crate::surface::SurfaceInfo;
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
use crate::PortholeError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RequireConfidence {
    #[default]
    Strong,
    Plausible,
    Weak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    Strong,
    Plausible,
    Weak,
}

impl Confidence {
    pub fn meets(self, required: RequireConfidence) -> bool {
        matches!(
            (self, required),
            (Confidence::Strong, _)
                | (Confidence::Plausible, RequireConfidence::Plausible | RequireConfidence::Weak)
                | (Confidence::Weak, RequireConfidence::Weak)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correlation {
    Tag,
    PidTree,
    Temporal,
    DocumentMatch,
    FrontmostChanged,
}

#[derive(Clone, Debug)]
pub struct ProcessLaunchSpec {
    pub app: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub require_confidence: RequireConfidence,
}

#[derive(Clone, Debug)]
pub struct LaunchOutcome {
    pub surface: SurfaceInfo,
    pub confidence: Confidence,
    pub correlation: Correlation,
    pub surface_was_preexisting: bool,
}

#[derive(Clone, Debug)]
pub struct Screenshot {
    pub png_bytes: Vec<u8>,
    pub window_bounds_points: Rect,
    pub content_bounds_points: Option<Rect>,
    pub scale: f64,
    pub captured_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError>;

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError>;

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError>;

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError>;

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError>;

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError>;

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError>;

    /// Wait for the condition to be satisfied. The pipeline layer wraps this
    /// in `tokio::time::timeout`; adapters may also respect the deadline
    /// internally for efficiency.
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<WaitOutcome, PortholeError>;

    /// Returns diagnostics appropriate for `wait_timeout` error payloads,
    /// given the last observed state of the condition. Called by the
    /// pipeline on timeout so the adapter can describe what it saw.
    async fn wait_last_observed(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError>;

    async fn attention(&self) -> Result<AttentionInfo, PortholeError>;

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError>;

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_meets_any_required() {
        assert!(Confidence::Strong.meets(RequireConfidence::Strong));
        assert!(Confidence::Strong.meets(RequireConfidence::Plausible));
        assert!(Confidence::Strong.meets(RequireConfidence::Weak));
    }

    #[test]
    fn plausible_fails_strong_requirement() {
        assert!(!Confidence::Plausible.meets(RequireConfidence::Strong));
        assert!(Confidence::Plausible.meets(RequireConfidence::Plausible));
    }

    #[test]
    fn weak_only_meets_weak() {
        assert!(!Confidence::Weak.meets(RequireConfidence::Plausible));
        assert!(Confidence::Weak.meets(RequireConfidence::Weak));
    }
}
