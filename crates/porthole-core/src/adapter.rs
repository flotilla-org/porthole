use std::time::Duration;

use async_trait::async_trait;

use crate::surface::SurfaceInfo;
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
