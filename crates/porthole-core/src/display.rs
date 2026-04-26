use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayId(String);

impl DisplayId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: DisplayId,
    pub bounds: Rect,
    pub scale: f64,
    pub primary: bool,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_id_is_transparent_string() {
        let id = DisplayId::new("disp_1");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"disp_1\"");
    }

    #[test]
    fn display_info_roundtrip() {
        let d = DisplayInfo {
            id: DisplayId::new("disp_1"),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
            },
            scale: 2.0,
            primary: true,
            focused: false,
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: DisplayInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, d);
    }
}
