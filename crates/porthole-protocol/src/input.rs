use porthole_core::input::{ClickButton, ClickSpec, CoordUnits, KeyEvent, Modifier, PointerMoveSpec, ScrollSpec};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyRequest {
    pub events: Vec<KeyEvent>,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyResponse {
    pub surface_id: String,
    pub events_sent: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextRequest {
    pub text: String,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextResponse {
    pub surface_id: String,
    pub chars_sent: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: ClickButton,
    #[serde(default = "default_count")]
    pub count: u8,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    /// Coordinate units for `x`, `y`. Default `logical` (unchanged behaviour).
    /// `physical` triggers daemon-side division by the surface's display scale.
    #[serde(default)]
    pub units: CoordUnits,
    #[serde(default)]
    pub session: Option<String>,
}

fn default_count() -> u8 {
    1
}

impl From<&ClickRequest> for ClickSpec {
    fn from(r: &ClickRequest) -> Self {
        ClickSpec {
            x: r.x,
            y: r.y,
            button: r.button,
            count: r.count,
            modifiers: r.modifiers.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClickResponse {
    pub surface_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollRequest {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
    /// Coordinate units for `x`, `y`. Default `logical` (unchanged behaviour).
    /// `physical` triggers daemon-side division by the surface's display scale.
    /// Note: only the position scales — `delta_x`/`delta_y` are wheel-line
    /// counts, not pixels, so they remain unchanged.
    #[serde(default)]
    pub units: CoordUnits,
    #[serde(default)]
    pub session: Option<String>,
}

impl From<&ScrollRequest> for ScrollSpec {
    fn from(r: &ScrollRequest) -> Self {
        ScrollSpec {
            x: r.x,
            y: r.y,
            delta_x: r.delta_x,
            delta_y: r.delta_y,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScrollResponse {
    pub surface_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointerMoveRequest {
    pub x: f64,
    pub y: f64,
    /// Coordinate units for `x`, `y`. Default `logical`. `physical` triggers
    /// daemon-side division by the surface's display scale.
    #[serde(default)]
    pub units: CoordUnits,
    #[serde(default)]
    pub session: Option<String>,
}

impl From<&PointerMoveRequest> for PointerMoveSpec {
    fn from(r: &PointerMoveRequest) -> Self {
        PointerMoveSpec { x: r.x, y: r.y }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PointerMoveResponse {
    pub surface_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_move_request_roundtrip() {
        let r = PointerMoveRequest {
            x: 12.0,
            y: 34.0,
            units: CoordUnits::Physical,
            session: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PointerMoveRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.x, 12.0);
        assert!(matches!(back.units, CoordUnits::Physical));
    }

    #[test]
    fn pointer_move_request_defaults_units_to_logical() {
        let json = r#"{ "x": 1.0, "y": 2.0 }"#;
        let r: PointerMoveRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(r.units, CoordUnits::Logical));
    }
}
