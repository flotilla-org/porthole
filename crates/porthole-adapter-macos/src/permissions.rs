#![cfg(target_os = "macos")]

use core_foundation::{base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString};
use porthole_core::{ErrorCode, PortholeError, permission::SystemPermissionStatus};

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

pub(crate) struct SupportedPermission {
    pub name: &'static str,
    pub purpose: &'static str,
    pub settings_path: &'static str,
    pub notes_base: &'static str,
    pub restart_required: bool,
    pub live_check: fn() -> bool,
    pub request_prompt: fn() -> bool,
}

/// Single source of truth for the macOS adapter's supported permissions.
/// Adding a permission: extend this slice and add an `ensure_*_granted`
/// facade in this module for call-site readability. The
/// `MacOsAdapter::prompted` array sizes itself from this constant — no
/// manual update needed there.
pub(crate) const SUPPORTED_PERMISSIONS: &[SupportedPermission] = &[
    SupportedPermission {
        name: "accessibility",
        purpose: "input injection and some wait conditions",
        settings_path: "System Settings → Privacy & Security → Accessibility",
        notes_base: "Open System Settings → Privacy & Security → Accessibility and enable porthole.",
        restart_required: true,
        live_check: ax_is_trusted_live,
        request_prompt: ax_request_prompt,
    },
    SupportedPermission {
        name: "screen_recording",
        purpose: "window screenshot capture and frame-diff waits",
        settings_path: "System Settings → Privacy & Security → Screen Recording",
        notes_base: "Open System Settings → Privacy & Security → Screen Recording and enable porthole.",
        restart_required: false,
        live_check: sr_is_granted_live,
        request_prompt: sr_request_prompt,
    },
];

pub(crate) fn lookup(name: &str) -> Option<&'static SupportedPermission> {
    SUPPORTED_PERMISSIONS.iter().find(|p| p.name == name)
}

pub(crate) fn unknown_name_error(name: &str) -> PortholeError {
    let supported: Vec<&str> = SUPPORTED_PERMISSIONS.iter().map(|p| p.name).collect();
    PortholeError::new(ErrorCode::InvalidArgument, format!("unknown system permission: {name}"))
        .with_details(serde_json::json!({ "supported_names": supported }))
}

pub async fn system_permissions() -> Result<Vec<SystemPermissionStatus>, PortholeError> {
    Ok(SUPPORTED_PERMISSIONS
        .iter()
        .map(|p| SystemPermissionStatus {
            name: p.name.into(),
            granted: (p.live_check)(),
            purpose: p.purpose.into(),
        })
        .collect())
}

/// Resolves the daemon's binary path for display in remediation blocks.
/// In dev builds this is the path inside `Portholed.app`.
pub fn daemon_binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

/// Human-readable Settings path for a named permission. Falls back to a
/// generic Privacy & Security entry for unknown names.
pub fn settings_path_for(name: &str) -> &'static str {
    lookup(name)
        .map(|p| p.settings_path)
        .unwrap_or("System Settings → Privacy & Security")
}

pub fn notes_for(name: &str) -> String {
    let Some(p) = lookup(name) else {
        return "Open System Settings → Privacy & Security and enable porthole.".to_string();
    };
    if p.restart_required {
        format!(
            "{} After granting, restart the daemon so the AX runtime initialises with the new trust state.",
            p.notes_base
        )
    } else {
        p.notes_base.to_string()
    }
}

pub fn requires_daemon_restart(name: &str) -> bool {
    lookup(name).map(|p| p.restart_required).unwrap_or(false)
}

pub(crate) fn is_granted(name: &str) -> Result<bool, PortholeError> {
    match lookup(name) {
        Some(p) => Ok((p.live_check)()),
        None => Err(unknown_name_error(name)),
    }
}

/// Try to open the OS prompt. Returns `Ok(())` on success (or no-op), or
/// `Err(reason)` if the process is not running in a bundle context where
/// TCC would actually open a dialog.
pub(crate) fn try_trigger_prompt(name: &str) -> Result<(), String> {
    let Some(p) = lookup(name) else {
        return Err(format!("unknown system permission: {name}"));
    };
    let is_bundle = std::env::current_exe()
        .ok()
        .and_then(|p| {
            p.ancestors()
                .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
                .map(|_| ())
        })
        .is_some();
    if !is_bundle {
        return Err("process is not running inside a .app bundle; TCC will not open a prompt. \
             Build via scripts/dev-bundle.sh and launch from the bundle."
            .to_string());
    }
    (p.request_prompt)();
    Ok(())
}

use porthole_protocol::system_permission::{Remediation, SystemPermissionNeededBody, SystemPermissionRequestFailedBody};

use crate::MacOsAdapter;

fn build_needed_body(name: &str) -> SystemPermissionNeededBody {
    let requires_restart = requires_daemon_restart(name);
    SystemPermissionNeededBody {
        permission: name.to_string(),
        remediation: Remediation {
            cli_command: "porthole onboard".to_string(),
            requires_daemon_restart: requires_restart,
            settings_path: settings_path_for(name).to_string(),
            binary_path: daemon_binary_path(),
        },
    }
}

pub(crate) fn build_request_failed_body(name: &str, reason: String) -> SystemPermissionRequestFailedBody {
    SystemPermissionRequestFailedBody {
        permission: name.to_string(),
        reason,
        settings_path: settings_path_for(name).to_string(),
        binary_path: daemon_binary_path(),
    }
}

/// Preflight for operations that require Accessibility. Triggers the OS
/// prompt on first miss per daemon process.
pub fn ensure_accessibility_granted(adapter: &MacOsAdapter) -> Result<(), PortholeError> {
    ensure_granted(adapter, "accessibility")
}

/// Preflight for operations that require Screen Recording. Triggers the OS
/// prompt on first miss per daemon process.
pub fn ensure_screen_recording_granted(adapter: &MacOsAdapter) -> Result<(), PortholeError> {
    ensure_granted(adapter, "screen_recording")
}

pub(crate) fn ensure_granted(adapter: &MacOsAdapter, name: &str) -> Result<(), PortholeError> {
    if is_granted(name)? {
        return Ok(());
    }

    // Try to trigger prompt only on first miss per process.
    if !adapter.was_prompted(name) {
        match try_trigger_prompt(name) {
            Ok(()) => {
                adapter.set_prompted(name);
            }
            Err(reason) => {
                let body = build_request_failed_body(name, reason);
                return Err(
                    PortholeError::new(ErrorCode::SystemPermissionRequestFailed, format!("cannot open prompt for {name}"))
                        .with_details(serde_json::to_value(body).unwrap_or_default()),
                );
            }
        }
    }

    let body = build_needed_body(name);
    Err(
        PortholeError::new(ErrorCode::SystemPermissionNeeded, format!("{name} permission required"))
            .with_details(serde_json::to_value(body).unwrap_or_default()),
    )
}

#[cfg(test)]
mod tests {
    use porthole_core::adapter::Adapter;

    use super::*;
    use crate::MacOsAdapter;

    #[tokio::test]
    #[ignore]
    async fn request_system_permission_prompt_accessibility_returns_outcome() {
        let adapter = MacOsAdapter::new();
        let outcome = adapter.request_system_permission_prompt("accessibility").await.expect("no-panic");
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
        let first = adapter.request_system_permission_prompt("accessibility").await.expect("no-panic");
        let second = adapter.request_system_permission_prompt("accessibility").await.expect("no-panic");
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

    #[tokio::test]
    #[ignore]
    async fn ensure_accessibility_returns_needed_when_missing() {
        let adapter = MacOsAdapter::new();
        if ax_is_trusted_live() {
            eprintln!("granted; test skipped");
            return;
        }
        let err = ensure_accessibility_granted(&adapter).expect_err("should error");
        assert_eq!(err.code, ErrorCode::SystemPermissionNeeded);
        let details = err.details.expect("details populated");
        assert_eq!(details["permission"], "accessibility");
        assert_eq!(details["remediation"]["cli_command"], "porthole onboard");
    }

    #[tokio::test]
    #[ignore]
    async fn ensure_accessibility_returns_ok_when_granted() {
        let adapter = MacOsAdapter::new();
        if !ax_is_trusted_live() {
            eprintln!("not granted; test skipped");
            return;
        }
        ensure_accessibility_granted(&adapter).expect("should be Ok when granted");
    }
}
