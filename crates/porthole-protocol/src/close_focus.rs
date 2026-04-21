use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CloseRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CloseResponse {
    pub surface_id: String,
    pub closed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FocusRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FocusResponse {
    pub surface_id: String,
    pub focused: bool,
}
