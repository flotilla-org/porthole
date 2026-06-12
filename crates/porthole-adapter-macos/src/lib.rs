#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::sync::Arc;

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot, VideoCaptureFramePublisher},
    attention::AttentionInfo,
    display::DisplayInfo,
    input::{ClickSpec, KeyEvent, PointerMoveSpec, ScrollSpec},
    permission::SystemPermissionStatus,
    surface::{PlatformSurfaceRef, SurfaceInfo},
    wait::{WaitCondition, WaitOutcome, WaitTimeout},
};
#[cfg(not(target_os = "macos"))]
use porthole_core::{permission::SystemPermissionPromptOutcome, wait::LastObserved};

#[cfg(target_os = "macos")]
pub mod artifact;
#[cfg(target_os = "macos")]
pub mod attention;
#[cfg(target_os = "macos")]
pub mod ax;
#[cfg(target_os = "macos")]
pub mod capture;
#[cfg(target_os = "macos")]
pub mod close_focus;
#[cfg(target_os = "macos")]
pub mod content_rect;
#[cfg(target_os = "macos")]
pub mod correlation;
#[cfg(target_os = "macos")]
pub mod cursor;
#[cfg(target_os = "macos")]
pub mod display;
#[cfg(target_os = "macos")]
pub mod enumerate;
#[cfg(target_os = "macos")]
pub mod ffi;
#[cfg(target_os = "macos")]
pub mod frame_diff;
#[cfg(target_os = "macos")]
pub mod input;
#[cfg(target_os = "macos")]
pub mod key_codes;
#[cfg(target_os = "macos")]
pub mod launch;
#[cfg(target_os = "macos")]
pub mod nsscreen;
#[cfg(target_os = "macos")]
pub mod permissions;
#[cfg(target_os = "macos")]
pub mod placement;
#[cfg(target_os = "macos")]
pub mod sck_capture;
#[cfg(target_os = "macos")]
pub mod sck_native;
#[cfg(target_os = "macos")]
pub mod search;
#[cfg(target_os = "macos")]
pub mod snapshot;
#[cfg(target_os = "macos")]
pub mod wait;
#[cfg(target_os = "macos")]
pub mod window_alive;

/// Stateless adapter — TCC trust state is loaded per-process by the macOS
/// runtime, not by us. Onboarding restarts the daemon between grants
/// (via `porthole onboard`'s launchctl-kickstart loop), so per-daemon-process
/// "have we prompted yet" bookkeeping isn't useful: each new daemon process
/// starts fresh and any earlier prompt belongs to a dead process.
#[derive(Default)]
pub struct MacOsAdapter {
    _private: (),
}

impl MacOsAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(target_os = "macos")]
#[async_trait]
impl Adapter for MacOsAdapter {
    fn name(&self) -> &'static str {
        "macos"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        launch::launch_process(self, spec).await
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        capture::screenshot(self, surface).await
    }

    async fn start_video_capture(
        &self,
        surface: &SurfaceInfo,
    ) -> Result<Box<dyn porthole_core::adapter::VideoCaptureSession>, PortholeError> {
        #[cfg(target_os = "macos")]
        {
            sck_capture::start_video_capture(self, surface).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = surface;
            Err(PortholeError::new(ErrorCode::AdapterUnsupported, "macOS adapter on non-macOS"))
        }
    }

    async fn start_video_capture_publisher(
        &self,
        surface: &SurfaceInfo,
        publisher: Arc<dyn VideoCaptureFramePublisher>,
    ) -> Result<Box<dyn porthole_core::adapter::VideoCaptureSession>, PortholeError> {
        #[cfg(target_os = "macos")]
        {
            sck_capture::start_video_capture_publisher(self, surface, publisher).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = surface;
            let _ = publisher;
            Err(PortholeError::new(ErrorCode::AdapterUnsupported, "macOS adapter on non-macOS"))
        }
    }

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
        input::key(self, surface, events).await
    }

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
        input::text(self, surface, text).await
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        input::click(self, surface, spec).await
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        input::scroll(self, surface, spec).await
    }

    async fn pointer_move(&self, surface: &SurfaceInfo, spec: &PointerMoveSpec) -> Result<(), PortholeError> {
        input::pointer_move(self, surface, spec).await
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::close(self, surface).await
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::focus(self, surface).await
    }

    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout> {
        wait::wait(self, surface, condition, deadline).await
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        attention::attention().await
    }

    async fn focused_platform_surface_ref(&self) -> Result<Option<PlatformSurfaceRef>, PortholeError> {
        Ok(attention::frontmost_cg_window_id().map(PlatformSurfaceRef::macos))
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        display::displays().await
    }

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError> {
        permissions::system_permissions().await
    }

    async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError> {
        if permissions::lookup(name).is_some() {
            permissions::ensure_granted(self, name)
        } else {
            Err(permissions::unknown_name_error(name))
        }
    }

    async fn request_system_permission_prompt(
        &self,
        name: &str,
    ) -> Result<porthole_core::permission::SystemPermissionPromptOutcome, PortholeError> {
        use porthole_core::permission::SystemPermissionPromptOutcome;

        // Name validation against our supported set. InvalidArgument carries
        // the supported list in details.
        let granted_before = permissions::is_granted(name)?;

        if !granted_before {
            // Attempt to open the OS prompt. TCC silently no-ops on
            // previously-denied permissions and on subsequent calls within
            // the same process; we don't track that here because the next
            // call goes through a freshly restarted daemon (per the onboard
            // flow's kickstart-between-grants design).
            if let Err(reason) = permissions::try_trigger_prompt(name) {
                let body = permissions::build_request_failed_body(name, reason);
                return Err(
                    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, "prompt rejected by OS")
                        .with_details(serde_json::to_value(body).unwrap_or_default()),
                );
            }
        }

        let granted_after = permissions::is_granted(name)?;
        let requires_daemon_restart = permissions::requires_daemon_restart(name);

        Ok(SystemPermissionPromptOutcome {
            permission: name.to_string(),
            granted_before,
            granted_after,
            requires_daemon_restart,
            notes: permissions::notes_for(name),
        })
    }

    async fn search(&self, query: &porthole_core::SearchQuery) -> Result<Vec<porthole_core::Candidate>, porthole_core::PortholeError> {
        search::search(self, query).await
    }

    async fn surface_alive(
        &self,
        pid: u32,
        platform_ref: &PlatformSurfaceRef,
    ) -> Result<Option<porthole_core::SurfaceInfo>, porthole_core::PortholeError> {
        let Some(cg_window_id) = platform_ref.as_macos_cg_window_id() else {
            return Ok(None);
        };
        window_alive::window_alive(self, pid, cg_window_id).await
    }

    async fn launch_artifact(
        &self,
        spec: &porthole_core::adapter::ArtifactLaunchSpec,
    ) -> Result<porthole_core::adapter::LaunchOutcome, porthole_core::PortholeError> {
        artifact::launch_artifact(self, spec).await
    }

    async fn place_surface(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
        rect: porthole_core::display::Rect,
    ) -> Result<(), porthole_core::PortholeError> {
        placement::place_surface(self, surface, rect).await
    }

    async fn snapshot_geometry(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::placement::GeometrySnapshot, porthole_core::PortholeError> {
        snapshot::snapshot_geometry(self, surface).await
    }

    async fn content_rect(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::content_rect::ContentRectInfo, porthole_core::PortholeError> {
        content_rect::content_rect(self, surface).await
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec![
            "launch_process",
            "screenshot",
            "input_key",
            "input_text",
            "input_click",
            "input_scroll",
            "input_pointer_move",
            "wait",
            "close",
            "focus",
            "attention",
            "attention_cursor",
            "attention_focused_app",
            "attention_focused_display",
            "attention_focused_surface",
            "displays",
            "search",
            "track",
            "launch_artifact",
            "placement",
            "replace",
            "auto_dismiss",
            "system_permission_prompt",
            "content_rect",
        ]
    }
}

#[cfg(not(target_os = "macos"))]
#[async_trait]
impl Adapter for MacOsAdapter {
    fn name(&self) -> &'static str {
        "macos"
    }

    async fn launch_process(&self, _spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        unsupported()
    }

    async fn screenshot(&self, _surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        unsupported()
    }

    async fn start_video_capture(
        &self,
        _surface: &SurfaceInfo,
    ) -> Result<Box<dyn porthole_core::adapter::VideoCaptureSession>, PortholeError> {
        unsupported()
    }

    async fn start_video_capture_publisher(
        &self,
        _surface: &SurfaceInfo,
        _publisher: Arc<dyn VideoCaptureFramePublisher>,
    ) -> Result<Box<dyn porthole_core::adapter::VideoCaptureSession>, PortholeError> {
        unsupported()
    }

    async fn key(&self, _surface: &SurfaceInfo, _events: &[KeyEvent]) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn text(&self, _surface: &SurfaceInfo, _text: &str) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn click(&self, _surface: &SurfaceInfo, _spec: &ClickSpec) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn scroll(&self, _surface: &SurfaceInfo, _spec: &ScrollSpec) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn pointer_move(&self, _surface: &SurfaceInfo, _spec: &PointerMoveSpec) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn close(&self, _surface: &SurfaceInfo) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn focus(&self, _surface: &SurfaceInfo) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn wait(
        &self,
        _surface: &SurfaceInfo,
        _condition: &WaitCondition,
        _deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout> {
        Err(WaitTimeout {
            last_observed: LastObserved::Presence { alive: false },
            elapsed_ms: 0,
        })
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        unsupported()
    }

    async fn focused_platform_surface_ref(&self) -> Result<Option<PlatformSurfaceRef>, PortholeError> {
        unsupported()
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        unsupported()
    }

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError> {
        Ok(Vec::new())
    }

    async fn ensure_system_permission(&self, _name: &str) -> Result<(), PortholeError> {
        unsupported()
    }

    async fn request_system_permission_prompt(&self, _name: &str) -> Result<SystemPermissionPromptOutcome, PortholeError> {
        unsupported()
    }

    async fn search(&self, _query: &porthole_core::SearchQuery) -> Result<Vec<porthole_core::Candidate>, PortholeError> {
        unsupported()
    }

    async fn surface_alive(
        &self,
        _pid: u32,
        _platform_ref: &PlatformSurfaceRef,
    ) -> Result<Option<porthole_core::SurfaceInfo>, PortholeError> {
        unsupported()
    }

    async fn launch_artifact(
        &self,
        _spec: &porthole_core::adapter::ArtifactLaunchSpec,
    ) -> Result<porthole_core::adapter::LaunchOutcome, PortholeError> {
        unsupported()
    }

    async fn place_surface(
        &self,
        _surface: &porthole_core::surface::SurfaceInfo,
        _rect: porthole_core::display::Rect,
    ) -> Result<(), porthole_core::PortholeError> {
        unsupported()
    }

    async fn snapshot_geometry(
        &self,
        _surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::placement::GeometrySnapshot, porthole_core::PortholeError> {
        unsupported()
    }

    async fn content_rect(
        &self,
        _surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::content_rect::ContentRectInfo, porthole_core::PortholeError> {
        unsupported()
    }

    fn capabilities(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

#[cfg(not(target_os = "macos"))]
fn unsupported<T>() -> Result<T, PortholeError> {
    Err(PortholeError::new(
        ErrorCode::AdapterUnsupported,
        "macOS adapter is unavailable on this platform",
    ))
}
