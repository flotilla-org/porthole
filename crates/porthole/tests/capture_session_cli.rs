use porthole::commands::capture_session::format_synthetic_session;
use porthole_protocol::capture_sessions::CreateCaptureSessionResponse;

#[test]
fn formats_synthetic_session_descriptor_for_attach_only_consumers() {
    let response = CreateCaptureSessionResponse {
        session_id: "capture-7".to_string(),
        source_id: 2,
        track_id: 3,
        status: "ready".to_string(),
        status_message: None,
        fd_socket_path: "/tmp/capture-transfer.sock".to_string(),
    };

    let rendered = format_synthetic_session("/tmp/porthole.sock", &response);

    assert!(rendered.contains("porthole_socket: /tmp/porthole.sock"));
    assert!(rendered.contains("session_id: capture-7"));
    assert!(rendered.contains("source_id: 2"));
    assert!(rendered.contains("track_id: 3"));
    assert!(rendered.contains("status: ready"));
    assert!(rendered.contains("capture-viewer-sdl --porthole-socket /tmp/porthole.sock --session-id capture-7"));
}
