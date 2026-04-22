use serde::{Deserialize, Serialize};

use crate::display::DisplayId;
use crate::surface::SurfaceId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttentionInfo {
    pub focused_surface_id: Option<SurfaceId>,
    pub focused_app_name: Option<String>,
    pub focused_display_id: Option<DisplayId>,
    pub cursor: CursorPos,
    pub recently_active_surface_ids: Vec<SurfaceId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CursorPos {
    pub x: f64,
    pub y: f64,
    pub display_id: Option<DisplayId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_info_roundtrip() {
        let a = AttentionInfo {
            focused_surface_id: Some(SurfaceId::from("surf_1")),
            focused_app_name: Some("com.example.app".into()),
            focused_display_id: Some(DisplayId::new("disp_1")),
            cursor: CursorPos { x: 100.0, y: 200.0, display_id: Some(DisplayId::new("disp_1")) },
            recently_active_surface_ids: vec![SurfaceId::from("surf_1"), SurfaceId::from("surf_2")],
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: AttentionInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn attention_info_cursor_display_id_serialises_as_string() {
        let a = AttentionInfo {
            focused_surface_id: None,
            focused_app_name: None,
            focused_display_id: Some(DisplayId::new("disp_2")),
            cursor: CursorPos { x: 50.0, y: 75.0, display_id: Some(DisplayId::new("disp_2")) },
            recently_active_surface_ids: vec![],
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"display_id\":\"disp_2\""), "json was: {json}");
    }

    #[test]
    fn attention_info_with_null_focus() {
        let a = AttentionInfo {
            focused_surface_id: None,
            focused_app_name: None,
            focused_display_id: None,
            cursor: CursorPos { x: 0.0, y: 0.0, display_id: None },
            recently_active_surface_ids: vec![],
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"focused_surface_id\":null"));
    }
}
