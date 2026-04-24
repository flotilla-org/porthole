#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::adapter::{
    Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot,
};
use porthole_core::attention::AttentionInfo;
use porthole_core::display::DisplayInfo;
use porthole_core::input::{ClickSpec, KeyEvent, ScrollSpec};
use porthole_core::permission::SystemPermissionStatus;
use porthole_core::surface::SurfaceInfo;
use porthole_core::wait::{WaitCondition, WaitOutcome, WaitTimeout};
use porthole_core::{ErrorCode, PortholeError};
use std::sync::atomic::{AtomicBool, Ordering};

pub mod artifact;
pub mod attention;
pub mod ax;
pub mod capture;
pub mod close_focus;
pub mod correlation;
pub mod cursor;
pub mod display;
pub mod enumerate;
pub mod ffi;
pub mod frame_diff;
pub mod input;
pub mod key_codes;
pub mod launch;
pub mod nsscreen;
pub mod permissions;
pub mod placement;
pub mod search;
pub mod snapshot;
pub mod wait;
pub mod window_alive;

pub struct MacOsAdapter {
    ax_prompted: AtomicBool,
    sr_prompted: AtomicBool,
}

impl MacOsAdapter {
    pub fn new() -> Self {
        Self {
            ax_prompted: AtomicBool::new(false),
            sr_prompted: AtomicBool::new(false),
        }
    }

    /// For preflight / request paths: mark a permission as having had its
    /// prompt API called. Returns the *previous* value (true if already
    /// prompted, false on first call).
    pub fn set_prompted(&self, name: &str) -> bool {
        match name {
            "accessibility" => self.ax_prompted.swap(true, Ordering::SeqCst),
            "screen_recording" => self.sr_prompted.swap(true, Ordering::SeqCst),
            _ => true, // unknown name: don't track, caller's problem
        }
    }

    /// Check without modifying.
    pub fn was_prompted(&self, name: &str) -> bool {
        match name {
            "accessibility" => self.ax_prompted.load(Ordering::SeqCst),
            "screen_recording" => self.sr_prompted.load(Ordering::SeqCst),
            _ => true,
        }
    }
}

impl Default for MacOsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Adapter for MacOsAdapter {
    fn name(&self) -> &'static str {
        "macos"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        launch::launch_process(spec).await
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        capture::screenshot(surface).await
    }

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
        input::key(surface, events).await
    }

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
        input::text(surface, text).await
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        input::click(surface, spec).await
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        input::scroll(surface, spec).await
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::close(surface).await
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        close_focus::focus(surface).await
    }

    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout> {
        wait::wait(surface, condition, deadline).await
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        attention::attention().await
    }

    async fn frontmost_window_id(&self) -> Result<Option<u32>, PortholeError> {
        Ok(attention::frontmost_cg_window_id())
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        display::displays().await
    }

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError> {
        permissions::system_permissions().await
    }

    async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError> {
        match name {
            "accessibility" => permissions::ensure_accessibility_granted(self),
            "screen_recording" => permissions::ensure_screen_recording_granted(self),
            _ => Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                format!("unknown system permission: {name}"),
            )
            .with_details(serde_json::json!({
                "supported_names": ["accessibility", "screen_recording"]
            }))),
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
        let was_prompted_before = self.was_prompted(name);

        if !granted_before {
            // Attempt to open the OS prompt.
            if let Err(reason) = permissions::try_trigger_prompt(name) {
                let body = permissions::build_request_failed_body(name, reason);
                return Err(
                    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, "prompt rejected by OS")
                        .with_details(serde_json::to_value(body).unwrap_or_default()),
                );
            }
            self.set_prompted(name);
        }

        let granted_after = permissions::is_granted(name)?;
        let prompt_triggered = !granted_before && !was_prompted_before;
        let requires_daemon_restart = permissions::requires_daemon_restart(name);

        Ok(SystemPermissionPromptOutcome {
            permission: name.to_string(),
            granted_before,
            granted_after,
            prompt_triggered,
            requires_daemon_restart,
            notes: permissions::notes_for(name, requires_daemon_restart),
        })
    }

    async fn search(
        &self,
        query: &porthole_core::SearchQuery,
    ) -> Result<Vec<porthole_core::Candidate>, porthole_core::PortholeError> {
        search::search(query).await
    }

    async fn window_alive(
        &self,
        pid: u32,
        cg_window_id: u32,
    ) -> Result<Option<porthole_core::SurfaceInfo>, porthole_core::PortholeError> {
        window_alive::window_alive(pid, cg_window_id).await
    }

    async fn launch_artifact(
        &self,
        spec: &porthole_core::adapter::ArtifactLaunchSpec,
    ) -> Result<porthole_core::adapter::LaunchOutcome, porthole_core::PortholeError> {
        artifact::launch_artifact(spec).await
    }

    async fn place_surface(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
        rect: porthole_core::display::Rect,
    ) -> Result<(), porthole_core::PortholeError> {
        placement::place_surface(surface, rect).await
    }

    async fn snapshot_geometry(
        &self,
        surface: &porthole_core::surface::SurfaceInfo,
    ) -> Result<porthole_core::placement::GeometrySnapshot, porthole_core::PortholeError> {
        snapshot::snapshot_geometry(surface).await
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec![
            "launch_process",
            "screenshot",
            "input_key",
            "input_text",
            "input_click",
            "input_scroll",
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
        ]
    }
}
