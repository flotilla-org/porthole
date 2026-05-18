use porthole_core::{display::Rect, input::CoordUnits, placement::PlacementSpec};
use serde::{Deserialize, Serialize};

/// Body for `POST /surfaces/{id}/place`.
///
/// Use `rect` for exact global screen coordinates, or `placement` for the
/// same display/anchor vocabulary accepted by launch and replace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<PlacementSpec>,
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
            rect: Some(Rect {
                x: 100.0,
                y: 200.0,
                w: 800.0,
                h: 600.0,
            }),
            placement: None,
            units: CoordUnits::Logical,
            session: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PlaceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.rect.unwrap().w, 800.0);
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
    fn place_request_accepts_placement_object_without_rect() {
        let json = r#"{ "placement": { "anchor": "focused_display" } }"#;
        let r: PlaceRequest = serde_json::from_str(json).unwrap();
        assert!(r.rect.is_none());
        assert!(r.placement.is_some());
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
