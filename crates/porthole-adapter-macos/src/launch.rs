use porthole_core::adapter::{LaunchOutcome, ProcessLaunchSpec};
use porthole_core::PortholeError;

pub async fn launch_process(_spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
    // Implemented in Task 18.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "macOS launch_process not yet implemented",
    ))
}
