use porthole_core::{content_rect::Descent, input::CoordUnits};
use serde::{Deserialize, Serialize};

/// Query string for `GET /surfaces/{id}/content-rect`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContentRectQuery {
    /// Coordinate units for the returned rect. Default `logical`.
    #[serde(default)]
    pub units: CoordUnits,
}

/// Body for `GET /surfaces/{id}/content-rect`. Returns the surface's inner
/// content rectangle in **window-local** coordinates. `ax_role` and `descent`
/// are debug-grade fields callers use to diagnose surprising results without
/// needing daemon logs — they are part of the wire contract.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentRectResponse {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub units: CoordUnits,
    pub ax_role: String,
    pub descent: Descent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_units_to_logical_when_field_missing() {
        let json = "{}";
        let q: ContentRectQuery = serde_json::from_str(json).unwrap();
        assert!(matches!(q.units, CoordUnits::Logical));
    }

    #[test]
    fn query_accepts_physical_units() {
        let json = r#"{ "units": "physical" }"#;
        let q: ContentRectQuery = serde_json::from_str(json).unwrap();
        assert!(matches!(q.units, CoordUnits::Physical));
    }

    #[test]
    fn response_roundtrip() {
        let r = ContentRectResponse {
            x: 0.0,
            y: 28.0,
            w: 1400.0,
            h: 872.0,
            units: CoordUnits::Logical,
            ax_role: "AXScrollArea".into(),
            descent: Descent::Contents,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ContentRectResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.y, 28.0);
        assert_eq!(back.ax_role, "AXScrollArea");
        assert!(matches!(back.descent, Descent::Contents));
        assert!(matches!(back.units, CoordUnits::Logical));
    }

    #[test]
    fn descent_serialises_snake_case() {
        let r = ContentRectResponse {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            units: CoordUnits::Logical,
            ax_role: "AXGroup".into(),
            descent: Descent::LargestChild,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"descent\":\"largest_child\""), "got: {json}");
    }
}
