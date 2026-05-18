use std::{sync::Arc, time::Duration};

use async_trait::async_trait;

pub use crate::display::Rect;
use crate::{
    ErrorCode, PortholeError,
    attention::AttentionInfo,
    content_rect::ContentRectInfo,
    display::DisplayInfo,
    input::{ClickSpec, KeyEvent, PointerMoveSpec, ScrollSpec},
    permission::{SystemPermissionPromptOutcome, SystemPermissionStatus},
    placement::GeometrySnapshot,
    search::{Candidate, SearchQuery},
    surface::SurfaceInfo,
    wait::{WaitCondition, WaitOutcome, WaitTimeout},
};

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
    pub force_place: bool,
}

#[derive(Clone, Debug)]
pub struct ArtifactLaunchSpec {
    pub path: std::path::PathBuf,
    pub require_confidence: RequireConfidence,
    pub require_fresh_surface: bool,
    pub force_place: bool,
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

    pub fn force_place(&self) -> bool {
        match self {
            LaunchSpec::Process(p) => p.force_place,
            LaunchSpec::Artifact(a) => a.force_place,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCapturePixelFormat {
    Bgra8Unorm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCaptureTimestampClock {
    Unknown,
    UnixTime,
    MediaTime,
    HostTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCaptureColorSpace {
    Unknown,
    Srgb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCaptureSyncKind {
    Unknown,
    CpuCopyComplete,
    SckSampleReady,
    NativeTimeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoCaptureDamageKind {
    Unknown,
    FullFrame,
    None,
    // TODO: expose inline and sidecar rect damage when the publisher path grows
    // variable-length damage metadata.
}

#[derive(Clone, Debug)]
pub struct VideoCaptureFrame {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub timestamp_clock: VideoCaptureTimestampClock,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: VideoCapturePixelFormat,
    pub color_space: VideoCaptureColorSpace,
    pub sync_kind: VideoCaptureSyncKind,
    pub damage_kind: VideoCaptureDamageKind,
    pub damage_base_sequence: u64,
    pub dropped_before_publish: u64,
    pub producer_drop_count: u64,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoCaptureFrameMetadata {
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub timestamp_clock: VideoCaptureTimestampClock,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: VideoCapturePixelFormat,
    pub color_space: VideoCaptureColorSpace,
    pub sync_kind: VideoCaptureSyncKind,
    pub damage_kind: VideoCaptureDamageKind,
    pub damage_base_sequence: u64,
    pub dropped_before_publish: u64,
    pub producer_drop_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoCaptureFrameView<'a> {
    pub metadata: VideoCaptureFrameMetadata,
    pub bytes: &'a [u8],
}

impl VideoCaptureFrame {
    #[must_use]
    pub const fn metadata(&self) -> VideoCaptureFrameMetadata {
        VideoCaptureFrameMetadata {
            sequence: self.sequence,
            timestamp_ns: self.timestamp_ns,
            timestamp_clock: self.timestamp_clock,
            width: self.width,
            height: self.height,
            stride: self.stride,
            pixel_format: self.pixel_format,
            color_space: self.color_space,
            sync_kind: self.sync_kind,
            damage_kind: self.damage_kind,
            damage_base_sequence: self.damage_base_sequence,
            dropped_before_publish: self.dropped_before_publish,
            producer_drop_count: self.producer_drop_count,
        }
    }

    #[must_use]
    pub fn as_view(&self) -> VideoCaptureFrameView<'_> {
        VideoCaptureFrameView {
            metadata: self.metadata(),
            bytes: &self.bytes,
        }
    }
}

#[cfg(test)]
mod video_capture_tests {
    use super::{
        VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCapturePixelFormat, VideoCaptureSyncKind,
        VideoCaptureTimestampClock,
    };

    #[test]
    fn owned_video_capture_frame_exposes_borrowed_view() {
        let frame = VideoCaptureFrame {
            sequence: 9,
            timestamp_ns: 123,
            timestamp_clock: VideoCaptureTimestampClock::MediaTime,
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: VideoCaptureColorSpace::Srgb,
            sync_kind: VideoCaptureSyncKind::SckSampleReady,
            damage_kind: VideoCaptureDamageKind::FullFrame,
            damage_base_sequence: 8,
            dropped_before_publish: 1,
            producer_drop_count: 2,
            bytes: vec![1, 2, 3, 4],
        };

        let view = frame.as_view();

        assert_eq!(view.metadata.sequence, 9);
        assert_eq!(view.metadata.damage_base_sequence, 8);
        assert_eq!(view.bytes, &[1, 2, 3, 4]);
    }
}

#[async_trait]
pub trait VideoCaptureSession: Send {
    async fn next_frame(&mut self) -> Result<Option<VideoCaptureFrame>, PortholeError>;
}

pub trait VideoCaptureFramePublisher: Send + Sync {
    fn publish_frame(&self, frame: VideoCaptureFrameView<'_>) -> Result<(), PortholeError>;
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError>;

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError>;

    async fn start_video_capture(&self, _surface: &SurfaceInfo) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
        Err(PortholeError::new(
            ErrorCode::AdapterUnsupported,
            "adapter does not support live video capture",
        ))
    }

    async fn start_video_capture_publisher(
        &self,
        _surface: &SurfaceInfo,
        _publisher: Arc<dyn VideoCaptureFramePublisher>,
    ) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
        Err(PortholeError::new(
            ErrorCode::AdapterUnsupported,
            "adapter does not support publisher-based live video capture",
        ))
    }

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError>;

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError>;

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError>;

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError>;

    /// Move the pointer to a window-local point. No button state change.
    /// Used by harnesses driving terminal mouse-reporting protocols
    /// (`DECSET ?1003 + ?1006 + ?1016`) that emit on motion alone.
    async fn pointer_move(&self, surface: &SurfaceInfo, spec: &PointerMoveSpec) -> Result<(), PortholeError>;

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

    /// Trigger the OS prompt for the named system permission. Returns a structured
    /// result with the grant state before/after and any restart requirement.
    /// Calling this for a permission that's already granted is a no-op that
    /// still returns the current state.
    ///
    /// `name` is a string matching one of the names the adapter advertises via
    /// `system_permissions()`. Unknown names return an `InvalidArgument` error
    /// with the supported names in details.
    async fn request_system_permission_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, PortholeError>;

    /// Preflight: verify the named system permission is granted. If not, the
    /// adapter may attempt to trigger an OS prompt as a side effect, then
    /// returns `Err(PortholeError)` with code `system_permission_needed` or
    /// `system_permission_request_failed`. Adapters that don't gate on
    /// OS permissions return `Ok(())`.
    async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError>;

    /// Enumerate candidate surfaces matching the query. Empty matches
    /// return `Ok(vec![])`, not an error.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError>;

    /// Return a live `SurfaceInfo` for the window identified by
    /// `(pid, cg_window_id)` if it still exists. The liveness check
    /// encompasses *all* windows, including hidden / minimized /
    /// other-Space windows — not just on-screen enumeration.
    async fn window_alive(&self, pid: u32, cg_window_id: u32) -> Result<Option<SurfaceInfo>, PortholeError>;

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

    /// Resolve the inner content rect of a surface, in **window-local logical**
    /// units. The pipeline converts to physical when the caller asks. Adapters
    /// return `ContentRectUnavailable` when the accessibility tree exposes the
    /// window but no usable content child.
    async fn content_rect(&self, surface: &SurfaceInfo) -> Result<ContentRectInfo, PortholeError>;

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
