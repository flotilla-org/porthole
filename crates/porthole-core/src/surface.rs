use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceId(String);

impl SurfaceId {
    pub fn new() -> Self {
        Self(format!("surf_{}", Uuid::new_v4().simple()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SurfaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Window,
    Tab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceState {
    Alive,
    Dead,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SurfaceInfo {
    pub id: SurfaceId,
    pub kind: SurfaceKind,
    pub state: SurfaceState,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
    pub pid: Option<u32>,
    pub parent_surface_id: Option<SurfaceId>,
}

impl SurfaceInfo {
    pub fn window(id: SurfaceId, pid: u32) -> Self {
        Self {
            id,
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: None,
            app_bundle: None,
            pid: Some(pid),
            parent_surface_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_surface_id_is_unique() {
        let a = SurfaceId::new();
        let b = SurfaceId::new();
        assert_ne!(a, b);
        assert!(a.as_str().starts_with("surf_"));
    }

    #[test]
    fn window_helper_sets_defaults() {
        let info = SurfaceInfo::window(SurfaceId::new(), 1234);
        assert_eq!(info.kind, SurfaceKind::Window);
        assert_eq!(info.state, SurfaceState::Alive);
        assert_eq!(info.pid, Some(1234));
        assert!(info.parent_surface_id.is_none());
    }

    #[test]
    fn surface_kind_roundtrips_as_snake_case() {
        let s = serde_json::to_string(&SurfaceKind::Window).unwrap();
        assert_eq!(s, "\"window\"");
        let k: SurfaceKind = serde_json::from_str("\"tab\"").unwrap();
        assert_eq!(k, SurfaceKind::Tab);
    }
}
