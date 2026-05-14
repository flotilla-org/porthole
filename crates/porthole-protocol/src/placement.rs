use porthole_core::{display::Rect, input::CoordUnits};
use serde::{Deserialize, Serialize};

/// Body for `POST /surfaces/{id}/place`. Explicit screen-coordinate rectangle;
/// anchor / display-target placement is launch-time only for now (phase 4 will
/// extend this with anchor support).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceRequest {
    pub rect: Rect,
    /// Coordinate units for `rect`. Default `logical` (unchanged behaviour).
    /// `physical` triggers daemon-side division by the surface's display
    /// scale across all four of `rect.x/y/w/h`.
    #[serde(default)]
    pub units: CoordUnits,
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
            units: CoordUnits::Logical,
            session: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PlaceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rect.w, 800.0);
        assert!(matches!(back.units, CoordUnits::Logical));
    }

    #[test]
    fn place_request_defaults_units_to_logical_when_field_missing() {
        // Wire compatibility: a pre-units client sends a JSON body without
        // `units`. Server must accept it and treat it as logical.
        let json = r#"{ "rect": { "x": 10, "y": 20, "w": 800, "h": 600 } }"#;
        let r: PlaceRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(r.units, CoordUnits::Logical));
    }

    #[test]
    fn place_request_accepts_physical_units() {
        let json = r#"{ "rect": { "x": 0, "y": 0, "w": 100, "h": 100 }, "units": "physical" }"#;
        let r: PlaceRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(r.units, CoordUnits::Physical));
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
