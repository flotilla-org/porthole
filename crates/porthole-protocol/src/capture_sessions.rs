use serde::{Deserialize, Serialize};

/// The launchd `MachServices` name portholed owns for the jackstay native
/// setup channel on macOS (ADR-0007). Consumers reach the XPC attach
/// service by looking this name up; the LaunchAgent plist written by
/// `porthole install` registers it.
pub const MACOS_NATIVE_ATTACH_MACH_SERVICE: &str = "work.flotilla.porthole.attach";

/// How a consumer reaches a macOS native (IOSurface/Metal) capture session.
/// Present only on native sessions; its absence means the CPU-shm fd-socket
/// path (`fd_socket_path`) applies. The viewer connects to `mach_service_name`
/// over XPC and presents `attach_token` as the bearer — a per-session
/// capability minted by portholed and conveyed here over the already
/// authenticated control socket, so the daemon never has to recover an agent
/// token's plaintext (it stores only hashes).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeCaptureInfo {
    pub mach_service_name: String,
    pub attach_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateCaptureSessionResponse {
    pub session_id: String,
    pub source_id: u64,
    pub track_id: u64,
    pub status: String,
    pub status_message: Option<String>,
    pub fd_socket_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeCaptureInfo>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeCaptureInfo>,
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

#[cfg(test)]
mod tests {
    use super::{CreateCaptureSessionResponse, NativeCaptureInfo};

    #[test]
    fn cpu_session_omits_the_native_field() {
        let cpu = CreateCaptureSessionResponse {
            session_id: "s".into(),
            source_id: 1,
            track_id: 2,
            status: "running".into(),
            status_message: None,
            fd_socket_path: "/tmp/x.sock".into(),
            native: None,
        };
        let json = serde_json::to_string(&cpu).unwrap();
        assert!(!json.contains("native"), "absent native field must not serialize: {json}");
        // And a payload without the field deserializes (back-compat / CPU path).
        let decoded: CreateCaptureSessionResponse =
            serde_json::from_str(r#"{"session_id":"s","source_id":1,"track_id":2,"status":"running","status_message":null,"fd_socket_path":"/tmp/x.sock"}"#).unwrap();
        assert!(decoded.native.is_none());
    }

    #[test]
    fn native_session_round_trips_the_attach_capability() {
        let native = CreateCaptureSessionResponse {
            session_id: "s".into(),
            source_id: 1,
            track_id: 2,
            status: "running".into(),
            status_message: None,
            fd_socket_path: String::new(),
            native: Some(NativeCaptureInfo {
                mach_service_name: "work.flotilla.porthole.attach".into(),
                attach_token: "pta_session.secret".into(),
            }),
        };
        let json = serde_json::to_string(&native).unwrap();
        let decoded: CreateCaptureSessionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.native, native.native);
    }
}
