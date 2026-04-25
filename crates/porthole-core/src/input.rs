use serde::{Deserialize, Serialize};

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
