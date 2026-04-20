use porthole_core::PortholeError;

#[derive(Clone, Debug)]
pub struct WindowRecord {
    pub cg_window_id: u32,
    pub owner_pid: i32,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    // Implemented in Task 17.
    Err(PortholeError::new(
        porthole_core::ErrorCode::CapabilityMissing,
        "list_windows not yet implemented",
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    Err(PortholeError::new(
        porthole_core::ErrorCode::AdapterUnsupported,
        "macOS adapter not supported on this platform",
    ))
}
