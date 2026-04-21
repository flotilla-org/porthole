use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub name: String,
    pub granted: bool,
    pub purpose: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_status_roundtrip() {
        let p = PermissionStatus {
            name: "accessibility".into(),
            granted: false,
            purpose: "input injection and some wait conditions".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PermissionStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}
