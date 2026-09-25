use serde::{Deserialize, Serialize};

/// The launchd `MachServices` name portholed owns for the jackstay native
/// setup channel on macOS (ADR-0007). Consumers reach the XPC attach
/// service by looking this name up; the LaunchAgent plist written by
/// `porthole install` registers it.
pub const MACOS_NATIVE_ATTACH_MACH_SERVICE: &str = "work.flotilla.porthole.attach";
pub const NATIVE_ATTACH_TRANSPORT_MACOS_XPC: u32 = 1;
pub const NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET: u32 = 2;
/// Windows: a Jackstay Local Endpoint (Wheelhouse ADR 0011) named pipe in
/// `Session` scope. `endpoint` is the logical endpoint name, rendered by
/// `jackstay::local::connect`. The consumer sends one
/// [`WindowsNativeAttachRequest`] line, reads one [`WindowsNativeAttachReply`]
/// line, and the same connection then carries Jackstay's D3D11 setup
/// (`serve_d3d11`) or CPU setup (`serve_cpu`), as the reply names. This is a
/// Porthole value: Jackstay's C `ft_native_attach_descriptor` has no Windows
/// transport.
pub const NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT: u32 = 3;

/// How a consumer reaches a native capture session.
/// Present only on native sessions; its absence means the CPU-shm fd-socket
/// path (`fd_socket_path`) applies. On macOS and Linux the transport and
/// endpoint map directly to `ft_native_attach_descriptor`: macOS uses XPC with
/// the Mach service name as endpoint, Linux uses a Unix-domain socket path.
/// Windows uses a Local Endpoint name
/// ([`NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT`]). `attach_token` is a
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

/// Optional body of `POST /capture-sessions/surfaces/{id}`: host capture
/// policy handed to the platform capture backend at start. Every field is
/// optional; `{}` (or no body) keeps the defaults. Only Windows native
/// sessions (`?native=true`) accept non-default values today; other paths
/// refuse them as `adapter_unsupported` rather than ignoring them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureSessionRequest {
    /// Draw the pointer into captured frames. Default: true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<bool>,
    /// Whether the system capture border may be shown. Default: `prefer_hidden`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<CaptureBorderPolicy>,
    /// Deliver frames at most this often, in milliseconds (1..=10000).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_update_interval_ms: Option<u32>,
    /// Published image size. Default: the source size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<CaptureOutputPolicy>,
}

impl CaptureSessionRequest {
    /// True when no field overrides a default.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self.cursor.is_none()
            && self.border.is_none()
            && self.min_update_interval_ms.is_none()
            && matches!(self.output, None | Some(CaptureOutputPolicy::Source))
    }
}

/// The capture border some systems draw around a captured window.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBorderPolicy {
    /// Leave the system's border.
    Show,
    /// Ask for no border; capture with one if the system refuses.
    PreferHidden,
    /// Ask for no border; fail the start if the system refuses.
    RequireHidden,
}

/// Published image size, fixed for the session's life.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureOutputPolicy {
    /// The captured region's own size; a resize publishes a new size.
    Source,
    /// Scale down, preserving aspect, to fit within this size; never up.
    Fit { width: u32, height: u32 },
    /// Always this size: the region is scaled to fit, preserving aspect,
    /// centred on black.
    Fixed { width: u32, height: u32 },
}

/// First line a consumer sends on a Windows native attach endpoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowsNativeAttachRequest {
    OpenNativeCapture { session_id: String, attach_token: String },
}

/// The publication an opened Windows native connection carries next.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowsNativePublication {
    /// Jackstay D3D11 setup: shared textures and an `ID3D11Fence`.
    D3d11,
    /// Jackstay CPU setup (the device cannot share fences).
    Cpu,
}

/// Reply line to [`WindowsNativeAttachRequest`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowsNativeAttachReply {
    /// Jackstay setup for `publication` follows on this connection. A later
    /// epoch (device loss) closes this publication; attach again.
    NativeCaptureOpened {
        publication: WindowsNativePublication,
        epoch: u64,
    },
    Rejected {
        message: String,
    },
}

/// Request output dimensions in pixels, independently of source geometry.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CaptureOutputRequest {
    pub width: u32,
    pub height: u32,
}

/// Backend acceptance does not assert that a new frame/pool is published yet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureOutputResponse {
    pub session_id: String,
    pub requested: CaptureOutputRequest,
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
    fn capture_policy_defaults_and_rejects_unknown_fields() {
        use super::{CaptureBorderPolicy, CaptureOutputPolicy, CaptureSessionRequest};
        let empty: CaptureSessionRequest = serde_json::from_str("{}").unwrap();
        assert!(empty.is_default());
        assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
        let policy: CaptureSessionRequest = serde_json::from_str(
            r#"{"cursor":false,"border":"require_hidden","min_update_interval_ms":20,"output":{"mode":"fit","width":640,"height":480}}"#,
        )
        .unwrap();
        assert_eq!(policy.cursor, Some(false));
        assert_eq!(policy.border, Some(CaptureBorderPolicy::RequireHidden));
        assert_eq!(policy.min_update_interval_ms, Some(20));
        assert_eq!(policy.output, Some(CaptureOutputPolicy::Fit { width: 640, height: 480 }));
        assert!(!policy.is_default());
        assert!(serde_json::from_str::<CaptureSessionRequest>(r#"{"cursour":false}"#).is_err());
        assert!(serde_json::from_str::<CaptureSessionRequest>(r#"{"output":{"mode":"stretch"}}"#).is_err());
    }

    #[test]
    fn windows_attach_lines_are_tagged() {
        use super::{WindowsNativeAttachReply, WindowsNativeAttachRequest, WindowsNativePublication};
        let request = WindowsNativeAttachRequest::OpenNativeCapture {
            session_id: "s".into(),
            attach_token: "t".into(),
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"op":"open_native_capture","session_id":"s","attach_token":"t"}"#
        );
        let reply = WindowsNativeAttachReply::NativeCaptureOpened {
            publication: WindowsNativePublication::D3d11,
            epoch: 1,
        };
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"op":"native_capture_opened","publication":"d3d11","epoch":1}"#
        );
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
