use std::path::Path;

use porthole_protocol::capture_sessions::CreateSyntheticCaptureSessionResponse;

use crate::client::{ClientError, DaemonClient};

pub struct SyntheticArgs<'a> {
    pub control_socket_path: &'a Path,
    pub json: bool,
}

pub async fn synthetic(client: &DaemonClient, args: SyntheticArgs<'_>) -> Result<(), ClientError> {
    let res: CreateSyntheticCaptureSessionResponse = client.post_json("/capture-sessions/synthetic", &serde_json::json!({})).await?;
    print_session_response("synthetic", client, args, &res)
}

pub async fn surface(client: &DaemonClient, surface_id: &str, args: SyntheticArgs<'_>) -> Result<(), ClientError> {
    let res: CreateSyntheticCaptureSessionResponse = client
        .post_json(&format!("/capture-sessions/surfaces/{surface_id}"), &serde_json::json!({}))
        .await?;
    print_session_response("surface", client, args, &res)
}

fn print_session_response(
    kind: &str,
    _client: &DaemonClient,
    args: SyntheticArgs<'_>,
    res: &CreateSyntheticCaptureSessionResponse,
) -> Result<(), ClientError> {
    if args.json {
        let text = serde_json::to_string_pretty(&serde_json::json!({
            "kind": kind,
            "porthole_socket": args.control_socket_path,
            "session_id": res.session_id,
            "source_id": res.source_id,
            "track_id": res.track_id,
            "fd_socket_path": res.fd_socket_path,
        }))
        .map_err(|error| ClientError::Local(format!("json encode: {error}")))?;
        println!("{text}");
    } else {
        print!("{}", format_synthetic_session(args.control_socket_path.display(), res));
    }
    Ok(())
}

pub fn format_synthetic_session(control_socket_path: impl std::fmt::Display, response: &CreateSyntheticCaptureSessionResponse) -> String {
    format!(
        "porthole_socket: {control_socket_path}\n\
         session_id: {}\n\
         source_id: {}\n\
         track_id: {}\n\
         fd_socket_path: {}\n\
         viewer: capture-viewer-sdl --porthole-socket {control_socket_path} --session-id {}\n",
        response.session_id, response.source_id, response.track_id, response.fd_socket_path, response.session_id
    )
}
