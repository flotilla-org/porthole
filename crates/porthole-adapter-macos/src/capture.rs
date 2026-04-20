use porthole_core::adapter::Screenshot;
use porthole_core::surface::SurfaceInfo;
use porthole_core::PortholeError;

pub async fn screenshot(_surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
    // Implemented in Task 19.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "macOS screenshot not yet implemented",
    ))
}
