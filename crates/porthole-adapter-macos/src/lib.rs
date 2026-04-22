#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::adapter::{
    Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot,
};
use porthole_core::attention::AttentionInfo;
use porthole_core::display::DisplayInfo;
use porthole_core::input::{ClickSpec, KeyEvent, ScrollSpec};
use porthole_core::permission::PermissionStatus;
use porthole_core::surface::SurfaceInfo;
use porthole_core::wait::{LastObserved, WaitCondition, WaitOutcome};
use porthole_core::PortholeError;

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
pub mod permissions;
pub mod search;
pub mod wait;
pub mod window_alive;

pub struct MacOsAdapter;

impl MacOsAdapter {
    pub fn new() -> Self {
        Self
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
    ) -> Result<WaitOutcome, PortholeError> {
        wait::wait(surface, condition).await
    }

    async fn wait_last_observed(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError> {
        wait::wait_last_observed(surface, condition).await
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

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError> {
        permissions::permissions().await
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
        ]
    }
}
