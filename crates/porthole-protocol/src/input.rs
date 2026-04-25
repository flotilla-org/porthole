use porthole_core::input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
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
