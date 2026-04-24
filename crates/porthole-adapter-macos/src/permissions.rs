#![cfg(target_os = "macos")]

use porthole_core::permission::SystemPermissionStatus;
use porthole_core::PortholeError;

unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn CGPreflightScreenCaptureAccess() -> u8;
}

pub async fn system_permissions() -> Result<Vec<SystemPermissionStatus>, PortholeError> {
    let ax = unsafe { AXIsProcessTrusted() } != 0;
    let scr = unsafe { CGPreflightScreenCaptureAccess() } != 0;
    Ok(vec![
        SystemPermissionStatus {
            name: "accessibility".into(),
            granted: ax,
            purpose: "input injection and some wait conditions".into(),
        },
        SystemPermissionStatus {
            name: "screen_recording".into(),
            granted: scr,
            purpose: "window screenshot capture and frame-diff waits".into(),
        },
    ])
}
