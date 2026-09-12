use serde::{Deserialize, Serialize};

/// The launchd `MachServices` name portholed owns for the jackstay native
/// setup channel on macOS (ADR-0007). Consumers reach the XPC attach
/// service by looking this name up; the LaunchAgent plist written by
/// `porthole install` registers it.
pub const MACOS_NATIVE_ATTACH_MACH_SERVICE: &str = "work.flotilla.porthole.attach";
pub const NATIVE_ATTACH_TRANSPORT_MACOS_XPC: u32 = 1;
pub const NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET: u32 = 2;

/// How a consumer reaches a native capture session.
/// Present only on native sessions; its absence means the CPU-shm fd-socket
/// path (`fd_socket_path`) applies. The transport and endpoint map directly to
/// `ft_native_attach_descriptor`: macOS uses XPC with the Mach service name as
/// endpoint, Linux uses a Unix-domain socket path. `attach_token` is a
/// per-session capability minted by portholed and conveyed here over the
/// already authenticated control socket, so the daemon never has to recover an
/// agent token's plaintext (it stores only hashes).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeCaptureInfo {
    pub transport_kind: u32,
    pub endpoint: String,
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
        let decoded: CreateCaptureSessionResponse = serde_json::from_str(
            r#"{"session_id":"s","source_id":1,"track_id":2,"status":"running","status_message":null,"fd_socket_path":"/tmp/x.sock"}"#,
        )
        .unwrap();
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
                transport_kind: super::NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
                endpoint: "work.flotilla.porthole.attach".into(),
                attach_token: "pta_session.secret".into(),
            }),
        };
        let json = serde_json::to_string(&native).unwrap();
        let decoded: CreateCaptureSessionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.native, native.native);
    }
}
