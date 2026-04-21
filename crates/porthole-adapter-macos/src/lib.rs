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

pub mod capture;
pub mod correlation;
pub mod enumerate;
pub mod ffi;
pub mod key_codes;
pub mod launch;

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

    async fn key(&self, _surface: &SurfaceInfo, _events: &[KeyEvent]) -> Result<(), PortholeError> {
        todo!("Batch E: macOS key input")
    }

    async fn text(&self, _surface: &SurfaceInfo, _text: &str) -> Result<(), PortholeError> {
        todo!("Batch E: macOS text input")
    }

    async fn click(&self, _surface: &SurfaceInfo, _spec: &ClickSpec) -> Result<(), PortholeError> {
        todo!("Batch E: macOS click input")
    }

    async fn scroll(&self, _surface: &SurfaceInfo, _spec: &ScrollSpec) -> Result<(), PortholeError> {
        todo!("Batch E: macOS scroll input")
    }

    async fn close(&self, _surface: &SurfaceInfo) -> Result<(), PortholeError> {
        todo!("Batch E: macOS close")
    }

    async fn focus(&self, _surface: &SurfaceInfo) -> Result<(), PortholeError> {
        todo!("Batch E: macOS focus")
    }

    async fn wait(&self, _surface: &SurfaceInfo, _condition: &WaitCondition) -> Result<WaitOutcome, PortholeError> {
        todo!("Batch E: macOS wait")
    }

    async fn wait_last_observed(
        &self,
        _surface: &SurfaceInfo,
        _condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError> {
        todo!("Batch E: macOS wait_last_observed")
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        todo!("Batch E: macOS attention")
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        todo!("Batch E: macOS displays")
    }

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError> {
        todo!("Batch E: macOS permissions")
    }
}
