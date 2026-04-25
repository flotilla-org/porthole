use porthole_core::permission::SystemPermissionPromptOutcome as CoreOutcome;
use serde::{Deserialize, Serialize};

/// Response body for `POST /system-permissions/request`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionPromptOutcome {
    pub permission: String,
    pub granted_before: bool,
    pub granted_after: bool,
    pub prompt_triggered: bool,
    pub requires_daemon_restart: bool,
    pub notes: String,
}

impl From<CoreOutcome> for SystemPermissionPromptOutcome {
    fn from(o: CoreOutcome) -> Self {
        let CoreOutcome {
            permission,
            granted_before,
            granted_after,
            prompt_triggered,
            requires_daemon_restart,
            notes,
        } = o;
        Self {
            permission,
            granted_before,
            granted_after,
            prompt_triggered,
            requires_daemon_restart,
            notes,
        }
    }
}

/// Body for `system_permission_needed`. Serialises into `WireError::details`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionNeededBody {
    pub permission: String,
    pub remediation: Remediation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Remediation {
    pub cli_command: String,
    pub requires_daemon_restart: bool,
    pub settings_path: String,
    pub binary_path: String,
}

/// Body for `system_permission_request_failed`. The daemon cannot open the
/// prompt; the user must grant manually in Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionRequestFailedBody {
    pub permission: String,
    pub reason: String,
    pub settings_path: String,
    pub binary_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_outcome_from_core_preserves_all_fields() {
        let core = CoreOutcome {
            permission: "accessibility".into(),
            granted_before: true,
            granted_after: false,
            prompt_triggered: true,
            requires_daemon_restart: true,
            notes: "n".into(),
        };
        let wire: SystemPermissionPromptOutcome = core.clone().into();
        assert_eq!(wire.permission, core.permission);
        assert_eq!(wire.granted_before, core.granted_before);
        assert_eq!(wire.granted_after, core.granted_after);
        assert_eq!(wire.prompt_triggered, core.prompt_triggered);
        assert_eq!(wire.requires_daemon_restart, core.requires_daemon_restart);
        assert_eq!(wire.notes, core.notes);
    }

    #[test]
    fn prompt_outcome_roundtrip() {
        let o = SystemPermissionPromptOutcome {
            permission: "accessibility".into(),
            granted_before: false,
            granted_after: false,
            prompt_triggered: true,
            requires_daemon_restart: true,
            notes: "Open System Settings...".into(),
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: SystemPermissionPromptOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn needed_body_roundtrip() {
        let b = SystemPermissionNeededBody {
            permission: "accessibility".into(),
            remediation: Remediation {
                cli_command: "porthole onboard".into(),
                requires_daemon_restart: true,
                settings_path: "System Settings → Privacy & Security → Accessibility".into(),
                binary_path: "/path/to/portholed".into(),
            },
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: SystemPermissionNeededBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }

    #[test]
    fn request_failed_body_roundtrip() {
        let b = SystemPermissionRequestFailedBody {
            permission: "screen_recording".into(),
            reason: "process is not in a bundle".into(),
            settings_path: "System Settings → Privacy & Security → Screen Recording".into(),
            binary_path: "/path/to/portholed".into(),
        };
        let json = serde_json::to_string(&b).unwrap();
        let back: SystemPermissionRequestFailedBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
    }
}
