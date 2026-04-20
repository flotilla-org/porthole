#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::adapter::{
    Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot,
};
use porthole_core::surface::SurfaceInfo;
use porthole_core::PortholeError;

pub mod capture;
pub mod correlation;
pub mod enumerate;
pub mod ffi;
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
}
