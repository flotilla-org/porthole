use std::time::Duration;

use async_trait::async_trait;

use crate::attention::AttentionInfo;
use crate::display::DisplayInfo;
pub use crate::display::Rect;
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::permission::SystemPermissionStatus;
use crate::placement::GeometrySnapshot;
use crate::search::{Candidate, SearchQuery};
use crate::surface::SurfaceInfo;
use crate::wait::{WaitCondition, WaitOutcome, WaitTimeout};
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
    pub require_fresh_surface: bool,
}

#[derive(Clone, Debug)]
pub struct ArtifactLaunchSpec {
    pub path: std::path::PathBuf,
    pub require_confidence: RequireConfidence,
    pub require_fresh_surface: bool,
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub enum LaunchSpec {
    Process(ProcessLaunchSpec),
    Artifact(ArtifactLaunchSpec),
}

impl LaunchSpec {
    pub fn require_confidence(&self) -> RequireConfidence {
        match self {
            LaunchSpec::Process(p) => p.require_confidence,
            LaunchSpec::Artifact(a) => a.require_confidence,
        }
    }

    pub fn require_fresh_surface(&self) -> bool {
        match self {
            LaunchSpec::Process(p) => p.require_fresh_surface,
            LaunchSpec::Artifact(a) => a.require_fresh_surface,
        }
    }
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

    /// Wait until the condition is satisfied, or `deadline` passes.
    ///
    /// Returns:
    /// - `Ok(WaitOutcome)` if the condition was satisfied.
    /// - `Err(WaitTimeout { last_observed, elapsed_ms })` if the deadline
    ///   passed first. The adapter populates `last_observed` with whatever
    ///   state it tracked during polling.
    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout>;

    async fn attention(&self) -> Result<AttentionInfo, PortholeError>;

    /// Returns the CGWindowID of the currently frontmost on-screen window, or
    /// `None` if it cannot be determined. Used by the attention route to resolve
    /// `focused_surface_id` against the handle store.
    async fn frontmost_window_id(&self) -> Result<Option<u32>, PortholeError>;

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError>;

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError>;

    /// Enumerate candidate surfaces matching the query. Empty matches
    /// return `Ok(vec![])`, not an error.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError>;

    /// Return a live `SurfaceInfo` for the window identified by
    /// `(pid, cg_window_id)` if it still exists. The liveness check
    /// encompasses *all* windows, including hidden / minimized /
    /// other-Space windows — not just on-screen enumeration.
    async fn window_alive(
        &self,
        pid: u32,
        cg_window_id: u32,
    ) -> Result<Option<SurfaceInfo>, PortholeError>;

    /// Launch a file artifact via OS default handler (macOS: `open <path>`).
    /// Correlates via DocumentMatch (strong) / FrontmostChanged (plausible) /
    /// Temporal (weak) as described in the spec §4.3.
    async fn launch_artifact(&self, spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    /// Apply a resolved placement rectangle in **global screen coordinates**
    /// to a tracked surface. The pipeline resolves on_display/anchor/geometry
    /// to a global rect and passes it here; adapter writes AXPosition + AXSize.
    async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError>;

    /// Read current geometry of a tracked surface, along with which display it's on.
    /// Returns display-local coords — caller (ReplacePipeline) uses both fields to
    /// inject inheritance into the replacement launch's placement.
    async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError>;

    /// The canonical string names of capabilities this adapter supports.
    /// Each entry corresponds to a verb/resource that the adapter can resolve
    /// non-trivially. Callers treat absence as "adapter cannot do this";
    /// presence means "calling this will have real effect on this platform."
    fn capabilities(&self) -> Vec<&'static str>;
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
