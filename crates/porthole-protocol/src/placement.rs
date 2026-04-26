use porthole_core::display::Rect;
use serde::{Deserialize, Serialize};

/// Body for `POST /surfaces/{id}/place`. Explicit screen-coordinate rectangle;
/// anchor / display-target placement is launch-time only for now (phase 4 will
/// extend this with anchor support).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceRequest {
    pub rect: Rect,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceResponse {
    pub surface_id: String,
    pub placed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_request_roundtrip() {
        let r = PlaceRequest {
            rect: Rect {
                x: 100.0,
                y: 200.0,
                w: 800.0,
                h: 600.0,
            },
            session: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PlaceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rect.w, 800.0);
    }

    #[test]
    fn place_response_serialises() {
        let r = PlaceResponse {
            surface_id: "surf_123".into(),
            placed: true,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"placed\":true"));
    }
}
