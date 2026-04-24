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
