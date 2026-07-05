use std::path::Path;

use porthole_protocol::capture_sessions::{
    CreateCaptureSessionResponse, NATIVE_ATTACH_TRANSPORT_MACOS_XPC, NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET,
};

use crate::client::{ClientError, DaemonClient};

pub struct CaptureSessionArgs<'a> {
    pub control_socket_path: &'a Path,
    pub json: bool,
}

pub async fn synthetic(client: &DaemonClient, args: CaptureSessionArgs<'_>) -> Result<(), ClientError> {
    let res: CreateCaptureSessionResponse = client.post_json("/capture-sessions/synthetic", &serde_json::json!({})).await?;
    print_session_response("synthetic", client, args, &res)
}

pub async fn surface(client: &DaemonClient, surface_id: &str, native: bool, args: CaptureSessionArgs<'_>) -> Result<(), ClientError> {
    let path = if native {
        format!("/capture-sessions/surfaces/{surface_id}?native=true")
    } else {
        format!("/capture-sessions/surfaces/{surface_id}")
    };
    let res: CreateCaptureSessionResponse = client.post_json(&path, &serde_json::json!({})).await?;
    print_session_response("surface", client, args, &res)
}

pub async fn close(client: &DaemonClient, session_id: &str) -> Result<(), ClientError> {
    client.delete_empty(&format!("/capture-sessions/{session_id}")).await?;
    println!("closed capture session {session_id}");
    Ok(())
}

fn print_session_response(
    kind: &str,
    _client: &DaemonClient,
    args: CaptureSessionArgs<'_>,
    res: &CreateCaptureSessionResponse,
) -> Result<(), ClientError> {
    if args.json {
        let text = serde_json::to_string_pretty(&serde_json::json!({
            "kind": kind,
            "porthole_socket": args.control_socket_path,
            "session_id": res.session_id,
            "source_id": res.source_id,
            "track_id": res.track_id,
            "status": res.status,
            "status_message": res.status_message,
            "fd_socket_path": res.fd_socket_path,
            "native": res.native,
        }))
        .map_err(|error| ClientError::Local(format!("json encode: {error}")))?;
        println!("{text}");
    } else {
        print!("{}", format_synthetic_session(args.control_socket_path.display(), res));
    }
    Ok(())
}

pub fn format_synthetic_session(control_socket_path: impl std::fmt::Display, response: &CreateCaptureSessionResponse) -> String {
    // Native sessions are consumed through the descriptor endpoint with the
    // per-session attach secret, not through the CPU fd socket.
    if let Some(native) = &response.native {
        let endpoint_label = match native.transport_kind {
            NATIVE_ATTACH_TRANSPORT_MACOS_XPC => "mach_service",
            NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET => "attach_socket",
            _ => "attach_endpoint",
        };
        return format!(
            "porthole_socket: {control_socket_path}\n\
             session_id: {}\n\
             source_id: {}\n\
             track_id: {}\n\
             status: {}\n\
             native_transport: {}\n\
             {endpoint_label}: {}\n\
             attach_token: {}\n\
             viewer: capture-viewer-sdl --native --transport-kind {} --endpoint {} --token {}\n",
            response.session_id,
            response.source_id,
            response.track_id,
            response.status,
            native.transport_kind,
            native.endpoint,
            native.attach_token,
            native.transport_kind,
            native.endpoint,
            native.attach_token,
        );
    }
    format!(
        "porthole_socket: {control_socket_path}\n\
         session_id: {}\n\
         source_id: {}\n\
         track_id: {}\n\
         status: {}\n\
         fd_socket_path: {}\n\
         viewer: capture-viewer-sdl --porthole-socket {control_socket_path} --session-id {}\n",
        response.session_id, response.source_id, response.track_id, response.status, response.fd_socket_path, response.session_id
    )
}
