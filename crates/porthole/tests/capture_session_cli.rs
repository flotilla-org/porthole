use porthole::commands::capture_session::format_synthetic_session;
use porthole_protocol::capture_sessions::{CreateCaptureSessionResponse, NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET, NativeCaptureInfo};

#[test]
fn formats_synthetic_session_descriptor_for_attach_only_consumers() {
    let response = CreateCaptureSessionResponse {
        session_id: "capture-7".to_string(),
        source_id: 2,
        track_id: 3,
        status: "ready".to_string(),
        status_message: None,
        fd_socket_path: "/tmp/capture-transfer.sock".to_string(),
        native: None,
    };

    let rendered = format_synthetic_session("/tmp/porthole.sock", &response);

    assert!(rendered.contains("porthole_socket: /tmp/porthole.sock"));
    assert!(rendered.contains("session_id: capture-7"));
    assert!(rendered.contains("source_id: 2"));
    assert!(rendered.contains("track_id: 3"));
    assert!(rendered.contains("status: ready"));
    assert!(rendered.contains("capture-viewer-sdl --porthole-socket /tmp/porthole.sock --session-id capture-7"));
}

#[test]
fn formats_native_session_descriptor_for_attach_only_consumers() {
    let response = CreateCaptureSessionResponse {
        session_id: "capture-8".to_string(),
        source_id: 2,
        track_id: 3,
        status: "ready".to_string(),
        status_message: None,
        fd_socket_path: String::new(),
        native: Some(NativeCaptureInfo {
            transport_kind: NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET,
            endpoint: "/tmp/native.sock".to_string(),
            attach_token: "ptas_secret".to_string(),
        }),
    };

    let rendered = format_synthetic_session("/tmp/porthole.sock", &response);

    assert!(rendered.contains("native_transport: 2"));
    assert!(rendered.contains("attach_socket: /tmp/native.sock"));
    assert!(rendered.contains("attach_token: ptas_secret"));
    assert!(rendered.contains("capture-viewer-sdl --native --transport-kind 2 --endpoint /tmp/native.sock --token ptas_secret"));
}
