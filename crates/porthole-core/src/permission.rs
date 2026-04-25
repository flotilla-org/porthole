use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SystemPermissionPromptOutcome {
    pub permission: String,
    pub granted_before: bool,
    pub granted_after: bool,
    pub prompt_triggered: bool,
    pub requires_daemon_restart: bool,
    pub notes: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_permission_status_roundtrip() {
        let p = SystemPermissionStatus {
            name: "accessibility".into(),
            granted: false,
            purpose: "input injection and some wait conditions".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: SystemPermissionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn prompt_outcome_roundtrip() {
        let o = SystemPermissionPromptOutcome {
            permission: "accessibility".into(),
            granted_before: false,
            granted_after: false,
            prompt_triggered: true,
            requires_daemon_restart: true,
            notes: "restart the daemon".into(),
        };
        let json = serde_json::to_string(&o).unwrap();
        let back: SystemPermissionPromptOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }
}
