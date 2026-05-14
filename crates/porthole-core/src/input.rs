use serde::{Deserialize, Serialize};

/// Coordinate unit system used by positional inputs (`click`, `scroll`, `place`).
///
/// macOS Cocoa APIs (and porthole's adapter layer) speak in **logical points**:
/// the unit a CGImage of a Retina screen reports as `width` / `bytes_per_row /
/// 4`'s ratio is *not* the same as what `NSWindow.frame` reports. A 2× Retina
/// display reports the same window as 1400 *logical* but 2800 *physical*
/// across these two surfaces.
///
/// Clients that source coordinates from physical-pixel sources — terminal
/// `CSI 16t` / `CSI 14t` reports (kitty etc. report physical px), porthole's
/// own `screenshot` output dimensions, raw `CGImage.width` — would otherwise
/// have to find their surface's display scale, divide, and pass logical.
/// `Physical` lets the daemon do that conversion at the boundary using the
/// surface's current display, so a window moving between mixed-DPI displays
/// can't desync the caller's cached scale factor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoordUnits {
    /// Logical points (Cocoa native). Default — preserves pre-flag behaviour.
    #[default]
    Logical,
    /// Physical pixels. Daemon converts by dividing by the surface's current
    /// display backing scale factor before applying.
    Physical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Modifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClickSpec {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: ClickButton,
    #[serde(default = "default_click_count")]
    pub count: u8,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

fn default_click_count() -> u8 {
    1
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScrollSpec {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_event_roundtrip() {
        let ev = KeyEvent {
            key: "KeyA".into(),
            modifiers: vec![Modifier::Cmd],
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: KeyEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn click_button_default_is_left() {
        let click = ClickSpec {
            x: 0.0,
            y: 0.0,
            button: ClickButton::default(),
            count: 1,
            modifiers: vec![],
        };
        assert_eq!(click.button, ClickButton::Left);
    }

    #[test]
    fn click_spec_deserializes_without_optional_fields() {
        let json = r#"{"x": 10.0, "y": 20.0}"#;
        let click: ClickSpec = serde_json::from_str(json).unwrap();
        assert_eq!(click.button, ClickButton::Left);
        assert_eq!(click.count, 1);
        assert!(click.modifiers.is_empty());
    }

    #[test]
    fn modifier_serializes_as_pascal_case() {
        let json = serde_json::to_string(&Modifier::Cmd).unwrap();
        assert_eq!(json, "\"Cmd\"");
    }
}
