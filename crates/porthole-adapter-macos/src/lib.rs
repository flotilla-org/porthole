#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use async_trait::async_trait;
use porthole_core::adapter::{Adapter, LaunchOutcome, ProcessLaunchSpec, Screenshot};
use porthole_core::surface::SurfaceInfo;
use porthole_core::PortholeError;

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

    async fn launch_process(&self, _spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        Err(PortholeError::new(
            porthole_core::ErrorCode::CapabilityMissing,
            "macOS launch_process not yet implemented",
        ))
    }

    async fn screenshot(&self, _surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        Err(PortholeError::new(
            porthole_core::ErrorCode::CapabilityMissing,
            "macOS screenshot not yet implemented",
        ))
    }
}
