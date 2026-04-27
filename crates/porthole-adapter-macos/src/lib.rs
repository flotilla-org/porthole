#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot},
    attention::AttentionInfo,
    display::DisplayInfo,
    input::{ClickSpec, KeyEvent, ScrollSpec},
    permission::SystemPermissionStatus,
    surface::SurfaceInfo,
    wait::{WaitCondition, WaitOutcome, WaitTimeout},
};

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
        let prompt_triggered = !granted_before;
        let requires_daemon_restart = permissions::requires_daemon_restart(name);

        Ok(SystemPermissionPromptOutcome {
            permission: name.to_string(),
            granted_before,
            granted_after,
            prompt_triggered,
            requires_daemon_restart,
            notes: permissions::notes_for(name),
        })
    }

    async fn search(&self, query: &porthole_core::SearchQuery) -> Result<Vec<porthole_core::Candidate>, porthole_core::PortholeError> {
        search::search(self, query).await
    }

    async fn window_alive(&self, pid: u32, cg_window_id: u32) -> Result<Option<porthole_core::SurfaceInfo>, porthole_core::PortholeError> {
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
