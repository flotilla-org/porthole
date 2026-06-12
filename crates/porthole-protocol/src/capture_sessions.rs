use serde::{Deserialize, Serialize};

/// The launchd `MachServices` name portholed owns for the jackstay native
/// setup channel on macOS (ADR-0007). Consumers reach the XPC attach
/// service by looking this name up; the LaunchAgent plist written by
/// `porthole install` registers it.
pub const MACOS_NATIVE_ATTACH_MACH_SERVICE: &str = "work.flotilla.porthole.attach";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateCaptureSessionResponse {
    pub session_id: String,
    pub source_id: u64,
    pub track_id: u64,
    pub status: String,
    pub status_message: Option<String>,
    pub fd_socket_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureSessionResponse {
    pub session_id: String,
    pub source_id: u64,
    pub track_id: u64,
    pub status: String,
    pub status_message: Option<String>,
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
    pub lease_id: u64,
    pub producer_cursor: u64,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
    pub pool_id: u64,
    pub slot_id: u32,
    pub payload_offset: u64,
    pub payload_len: u64,
    pub payload_map_len: u64,
    pub clock_domain: String,
    pub color_space: String,
    pub sync_kind: String,
    pub damage_kind: String,
    pub damage_base_sequence: u64,
    pub dropped_before_publish: u32,
    pub producer_drop_count: u64,
    pub evicted_count: u64,
    pub consumer_skipped_count: u64,
    pub len: u64,
}
