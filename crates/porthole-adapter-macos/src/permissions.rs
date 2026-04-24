#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use porthole_core::permission::SystemPermissionStatus;
use porthole_core::{ErrorCode, PortholeError};

unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> u8;
    fn CGPreflightScreenCaptureAccess() -> u8;
    fn CGRequestScreenCaptureAccess() -> u8;
}

/// Named constant from AppKit: `kAXTrustedCheckOptionPrompt`.
fn ax_trusted_check_option_prompt_key() -> CFString {
    CFString::from_static_string("AXTrustedCheckOptionPrompt")
}

fn ax_is_trusted_live() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

fn sr_is_granted_live() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() != 0 }
}

/// Calls `AXIsProcessTrustedWithOptions` with `kAXTrustedCheckOptionPrompt: true`.
/// The OS may show a dialog on the first call per process for a given bundle
/// identity; subsequent calls are silent. Returns whether the process is
/// currently trusted, per AX's own return value.
fn ax_request_prompt() -> bool {
    let key = ax_trusted_check_option_prompt_key();
    let value = CFBoolean::true_value();
    let pairs = [(key.as_CFType(), value.as_CFType())];
    let dict = CFDictionary::from_CFType_pairs(&pairs);
    unsafe { AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const _) != 0 }
}

fn sr_request_prompt() -> bool {
    unsafe { CGRequestScreenCaptureAccess() != 0 }
}

pub async fn system_permissions() -> Result<Vec<SystemPermissionStatus>, PortholeError> {
    let ax = ax_is_trusted_live();
    let scr = sr_is_granted_live();
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

/// Resolves the daemon's binary path for display in remediation blocks.
/// In dev builds this is the path inside `Portholed.app`.
pub fn daemon_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

/// Human-readable Settings path for a named permission.
pub fn settings_path_for(name: &str) -> &'static str {
    match name {
        "accessibility" => "System Settings → Privacy & Security → Accessibility",
        "screen_recording" => "System Settings → Privacy & Security → Screen Recording",
        _ => "System Settings → Privacy & Security",
    }
}

pub fn notes_for(name: &str, requires_restart: bool) -> String {
    let base = match name {
        "accessibility" => "Open System Settings → Privacy & Security → Accessibility and enable porthole.",
        "screen_recording" => "Open System Settings → Privacy & Security → Screen Recording and enable porthole.",
        _ => "Open System Settings → Privacy & Security and enable porthole.",
    };
    if requires_restart {
        format!("{base} After granting, restart the daemon so the AX runtime initialises with the new trust state.")
    } else {
        base.to_string()
    }
}

pub fn requires_daemon_restart(name: &str) -> bool {
    matches!(name, "accessibility")
}

pub(crate) fn is_granted(name: &str) -> Result<bool, PortholeError> {
    match name {
        "accessibility" => Ok(ax_is_trusted_live()),
        "screen_recording" => Ok(sr_is_granted_live()),
        _ => Err(PortholeError::new(
            ErrorCode::InvalidArgument,
            format!("unknown system permission: {name}"),
        )
        .with_details(serde_json::json!({
            "supported_names": ["accessibility", "screen_recording"]
        }))),
    }
}

/// Try to open the OS prompt. Returns `Ok(())` on success (or no-op), or
/// `Err(reason)` if the process is not running in a bundle context where
/// TCC would actually open a dialog.
pub(crate) fn try_trigger_prompt(name: &str) -> Result<(), String> {
    let is_bundle = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.ancestors()
                .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
                .map(|_| ())
        })
        .is_some();
    if !is_bundle {
        return Err(
            "process is not running inside a .app bundle; TCC will not open a prompt. \
             Build via scripts/dev-bundle.sh and launch from the bundle."
                .to_string(),
        );
    }
    match name {
        "accessibility" => {
            ax_request_prompt();
            Ok(())
        }
        "screen_recording" => {
            sr_request_prompt();
            Ok(())
        }
        _ => Err(format!("unknown system permission: {name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MacOsAdapter;
    use porthole_core::adapter::Adapter;

    #[tokio::test]
    #[ignore]
    async fn request_system_permission_prompt_accessibility_returns_outcome() {
        let adapter = MacOsAdapter::new();
        let outcome = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        assert_eq!(outcome.permission, "accessibility");
        assert_eq!(outcome.granted_before, ax_is_trusted_live());
        assert!(outcome.requires_daemon_restart);
    }

    #[tokio::test]
    #[ignore]
    async fn prompt_bookkeeping_flips_on_first_call_only() {
        let adapter = MacOsAdapter::new();
        if ax_is_trusted_live() {
            eprintln!("accessibility already granted; test skipped");
            return;
        }
        let first = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        let second = adapter
            .request_system_permission_prompt("accessibility")
            .await
            .expect("no-panic");
        assert!(first.prompt_triggered, "first call should trigger prompt");
        assert!(!second.prompt_triggered, "second call should not re-trigger");
    }

    #[tokio::test]
    async fn unknown_permission_name_returns_invalid_argument() {
        let adapter = MacOsAdapter::new();
        let err = adapter
            .request_system_permission_prompt("coffee_grinder")
            .await
            .expect_err("should reject unknown name");
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        let details = err.details.expect("details populated");
        let supported = details.get("supported_names").and_then(|v| v.as_array()).unwrap();
        assert!(supported.iter().any(|v| v == "accessibility"));
    }
}
