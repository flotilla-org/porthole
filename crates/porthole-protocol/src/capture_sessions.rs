use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateSyntheticCaptureSessionResponse {
    pub session_id: String,
    pub source_id: u64,
    pub track_id: u64,
    pub fd_socket_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureSessionResponse {
    pub session_id: String,
    pub source_id: u64,
    pub track_id: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
    pub fd_socket_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatestVideoFrameRequest {
    pub session_id: String,
    pub track_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LatestVideoFrameResponse {
    pub session_id: String,
    pub track_id: u64,
    pub sequence: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
    pub len: usize,
}
