use serde::{Deserialize, Serialize};

use crate::SurfaceId;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScreenshotRequest {
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScreenshotResponse {
    pub surface_id: SurfaceId,
    pub png_base64: String,
    pub window_bounds: Rect,
    pub content_bounds: Option<Rect>,
    pub scale: f64,
    pub captured_at_unix_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
