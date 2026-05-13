use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateCaptureSessionResponse {
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
    pub timestamp_ns: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
    pub pool_id: u64,
    pub slot_id: u64,
    pub slot_generation: u64,
    pub payload_offset: u64,
    pub payload_len: u64,
    pub payload_map_len: u64,
    pub clock_domain: String,
    pub color_space: String,
    pub sync_kind: String,
    pub damage_kind: String,
    pub damage_base_sequence: u64,
    pub dropped_before_publish: u64,
    pub producer_drop_count: u64,
    pub evicted_count: u64,
    pub consumer_skipped_count: u64,
    pub len: usize,
}
