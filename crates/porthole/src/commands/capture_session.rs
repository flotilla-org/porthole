use std::path::Path;

use porthole_protocol::capture_sessions::{
    CaptureSessionRequest, CaptureSessionResponse, CreateCaptureSessionResponse, NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
    NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET, NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT,
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

pub async fn surface(
    client: &DaemonClient,
    surface_id: &str,
    native: bool,
    policy: &CaptureSessionRequest,
    args: CaptureSessionArgs<'_>,
) -> Result<(), ClientError> {
    let path = if native {
        format!("/capture-sessions/surfaces/{surface_id}?native=true")
    } else {
        format!("/capture-sessions/surfaces/{surface_id}")
    };
    let res: CreateCaptureSessionResponse = client.post_json(&path, policy).await?;
    print_session_response("surface", client, args, &res)
}

/// Print a session's current status (lifecycle, size, and backend detail
/// such as a desktop-unavailable pause or a device-loss epoch).
pub async fn status(client: &DaemonClient, session_id: &str, json: bool) -> Result<(), ClientError> {
    let res: CaptureSessionResponse = client.get_json(&format!("/capture-sessions/{session_id}")).await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&res).map_err(|error| ClientError::Local(format!("json encode: {error}")))?
        );
    } else {
        println!(
            "session_id: {}\nstatus: {}\nsize: {}x{}\nmessage: {}",
            res.session_id,
            res.status,
            res.width,
            res.height,
            res.status_message.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

pub async fn configure(client: &DaemonClient, session_id: &str, width: u32, height: u32, json: bool) -> Result<(), ClientError> {
    use porthole_protocol::capture_sessions::{CaptureOutputRequest, CaptureOutputResponse};
    let res: CaptureOutputResponse = client
        .post_json(
            &format!("/capture-sessions/{session_id}/output"),
            &CaptureOutputRequest { width, height },
        )
        .await?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&res).map_err(|error| ClientError::Local(format!("json encode: {error}")))?
        );
    } else {
        println!(
            "capture session {} accepted output request {}×{} pixels; published dimensions are available from GET /capture-sessions/{}",
            res.session_id, res.requested.width, res.requested.height, res.session_id
        );
    }
    Ok(())
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
            NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT => "local_endpoint",
            _ => "attach_endpoint",
        };
        if native.transport_kind == NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT {
            return format!(
                "porthole_socket: {control_socket_path}\n\
                 session_id: {}\n\
                 source_id: {}\n\
                 track_id: {}\n\
                 status: {}\n\
                 native_transport: {}\n\
                 {endpoint_label}: {}\n\
                 attach_token: {}\n",
                response.session_id,
                response.source_id,
                response.track_id,
                response.status,
                native.transport_kind,
                native.endpoint,
                native.attach_token,
            );
        }
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
