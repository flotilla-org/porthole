//! `porthole publications`: list publications, export one for a remote
//! consumer, republish a forwarded export locally, and `attach`, which does the
//! whole cross-host sequence over SSH until Tender provides the forwarding.

use std::{path::PathBuf, process::Stdio, time::Duration};

use porthole_core::error::ErrorCode;
use porthole_protocol::{
    capture_sessions::CreateCaptureSessionResponse,
    publications::{
        ChromaPolicy, CreateExportRequest, ExportResponse, Identities, ListPublicationsResponse, PublicationResponse, RepublishRequest,
        RepublishResponse,
    },
    search::{SearchQuery, SearchRequest, SearchResponse, TrackRequest, TrackResponse},
};
use tokio::process::{Child, Command};

use crate::client::{ClientError, DaemonClient};

fn viewer_hints(publication: &PublicationResponse) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(native) = &publication.native {
        hints.push(format!(
            "viewer: capture-viewer-sdl --native --mach-service {} --token {}",
            native.endpoint, native.attach_token
        ));
    }
    if let Some(socket) = &publication.cpu_socket {
        let input = publication
            .input_socket
            .as_ref()
            .map_or(String::new(), |i| format!(" --input-socket {i}"));
        hints.push(format!("cpu viewer: capture-viewer-sdl --cpu-socket {socket}{input}"));
        hints.push(format!("katzensteg: katzensteg jackstay-source {socket}"));
    } else if let Some(input) = &publication.input_socket {
        hints.push(format!("input socket: {input}"));
    }
    hints
}

fn print_publication(publication: &PublicationResponse) {
    println!(
        "{}  {}  {}  {}x{}  source={}  {}",
        publication.publication_id,
        publication.kind,
        publication.status,
        publication.width,
        publication.height,
        publication.identities.source,
        publication.status_message.as_deref().unwrap_or("")
    );
}

pub async fn list(client: &DaemonClient, json: bool) -> Result<(), ClientError> {
    let response: ListPublicationsResponse = client.get_json("/publications").await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    for publication in &response.publications {
        print_publication(publication);
        for hint in viewer_hints(publication) {
            println!("  {hint}");
        }
    }
    Ok(())
}

pub async fn export(
    client: &DaemonClient,
    publication_id: &str,
    chroma: ChromaPolicy,
    bitrate_bps: Option<u32>,
    input: bool,
    json: bool,
) -> Result<(), ClientError> {
    let response: ExportResponse = client
        .post_json(
            &format!("/publications/{publication_id}/exports"),
            &CreateExportRequest {
                chroma,
                bitrate_bps,
                input,
            },
        )
        .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_export(&response);
    }
    Ok(())
}

fn print_export(response: &ExportResponse) {
    println!(
        "export_id: {}\npublication_id: {}\nstatus: {}{}\nmedia_socket: {}\ncontrol_socket: {}\nlink_token: {}",
        response.export_id,
        response.publication_id,
        response.status,
        response.status_message.as_deref().map_or(String::new(), |m| format!(" ({m})")),
        response.media_socket,
        response.control_socket,
        response.link_token
    );
    if let Some(d) = &response.decision {
        println!("decision: {:?} {:?} hardware={} ({})", d.codec, d.chroma, d.hardware, d.reason);
    }
}

pub async fn export_status(client: &DaemonClient, publication_id: &str, export_id: &str, json: bool) -> Result<(), ClientError> {
    let response: ExportResponse = client
        .get_json(&format!("/publications/{publication_id}/exports/{export_id}"))
        .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_export(&response);
        if let Some(report) = &response.report {
            println!("report: {report}");
        }
    }
    Ok(())
}

pub async fn export_close(client: &DaemonClient, publication_id: &str, export_id: &str) -> Result<(), ClientError> {
    let response: ExportResponse = client
        .delete_json(&format!("/publications/{publication_id}/exports/{export_id}"))
        .await?;
    println!("closed export {} ({})", response.export_id, response.status);
    Ok(())
}

pub async fn republish(client: &DaemonClient, request: &RepublishRequest, json: bool) -> Result<RepublishResponse, ClientError> {
    let response: RepublishResponse = client.post_json("/publications/republish", request).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_publication(&response.publication);
        if let Some(d) = &response.decision {
            println!("decision: {:?} {:?} hardware={} ({})", d.codec, d.chroma, d.hardware, d.reason);
        }
        for hint in viewer_hints(&response.publication) {
            println!("{hint}");
        }
    }
    Ok(response)
}

pub async fn close_republication(client: &DaemonClient, publication_id: &str) -> Result<(), ClientError> {
    client.delete_empty(&format!("/publications/{publication_id}")).await?;
    println!("closed republication {publication_id}");
    Ok(())
}

// ---- attach: the cross-host sequence over SSH -------------------------------------

pub struct AttachArgs {
    pub host: String,
    /// The remote daemon's runtime directory; resolved from the remote per-user
    /// temp dir when absent.
    pub remote_runtime_dir: Option<String>,
    /// Falls back to `PORTHOLE_REMOTE_AGENT_TOKEN`.
    pub remote_agent_token: Option<String>,
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,
    pub chroma: ChromaPolicy,
    pub bitrate_bps: Option<u32>,
    /// Also serve the republication over a generic CPU setup socket.
    pub cpu: bool,
    /// Carry an input channel back to the captured surface (needs Drive on
    /// the remote surface).
    pub input: bool,
    pub json: bool,
    /// Keep the forwards and the republication alive until interrupted.
    pub hold: bool,
}

/// An `ssh -N -L` process forwarding Unix sockets; killed on drop.
struct Forward {
    child: Child,
}

impl Forward {
    async fn start(host: &str, pairs: &[(PathBuf, String)]) -> Result<Self, ClientError> {
        let mut command = Command::new("ssh");
        command
            .arg("-o")
            .arg("StreamLocalBindUnlink=yes")
            .arg("-o")
            .arg("ExitOnForwardFailure=yes")
            .arg("-c")
            .arg("aes128-gcm@openssh.com")
            .arg("-N");
        for (local, remote) in pairs {
            command.arg("-L").arg(format!("{}:{}", local.display(), remote));
        }
        command
            .arg(host)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let child = command.spawn().map_err(|e| ClientError::Local(format!("ssh: {e}")))?;
        // wait for the local sockets to appear
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while pairs.iter().any(|(local, _)| !local.exists()) {
            if tokio::time::Instant::now() > deadline {
                return Err(ClientError::Local("ssh forward did not come up within 15 s".into()));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(Self { child })
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Retries an operation while the remote reports `agent_permission_needed`,
/// which means an operator on that host has a request to approve.
async fn with_remote_approval<T, F, Fut>(host: &str, what: &str, mut op: F) -> Result<T, ClientError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ClientError>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut told = false;
    loop {
        match op().await {
            Err(ClientError::Api(e)) if e.code == ErrorCode::AgentPermissionNeeded && tokio::time::Instant::now() < deadline => {
                if !told {
                    eprintln!("waiting for an operator on {host} to approve {what} (porthole agents requests / approve)");
                    told = true;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            other => return other,
        }
    }
}

async fn remote_runtime_dir(host: &str) -> Result<String, ClientError> {
    // The daemon listens under the per-user temp dir of the GUI session, which
    // an SSH login does not set in TMPDIR; getconf resolves it.
    let output = Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, "getconf DARWIN_USER_TEMP_DIR; id -u"])
        .output()
        .await
        .map_err(|e| ClientError::Local(format!("ssh {host}: {e}")))?;
    if !output.status.success() {
        return Err(ClientError::Local(format!(
            "ssh {host}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let tmp = lines.next().unwrap_or("").trim().trim_end_matches('/');
    let uid = lines.next().unwrap_or("").trim();
    if tmp.is_empty() || uid.is_empty() {
        return Err(ClientError::Local(format!("could not resolve the porthole runtime dir on {host}")));
    }
    Ok(format!("{tmp}/porthole-{uid}"))
}

pub async fn attach(local: &DaemonClient, args: AttachArgs) -> Result<(), ClientError> {
    // Short names throughout: Unix socket paths are limited to 104 bytes. The
    // directory is private to this user and unpredictable, so another local
    // user cannot pre-create it under the forwarded sockets.
    let suffix = jackstay_graph::mint_token().map_err(|e| ClientError::Local(format!("work dir: {e}")))?;
    let work = std::env::temp_dir().join(format!("pa{}", &suffix[..8]));
    #[cfg(unix)]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&work).map_err(|e| ClientError::Local(format!("work dir: {e}")))?;
    let remote_dir = match &args.remote_runtime_dir {
        Some(dir) => dir.trim_end_matches('/').to_owned(),
        None => remote_runtime_dir(&args.host).await?,
    };
    let control_local = work.join("p");
    let _control_forward = Forward::start(&args.host, &[(control_local.clone(), format!("{remote_dir}/porthole.sock"))]).await?;
    let remote_agent_token = args
        .remote_agent_token
        .clone()
        .or_else(|| std::env::var("PORTHOLE_REMOTE_AGENT_TOKEN").ok())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ClientError::Local("--remote-agent-token or PORTHOLE_REMOTE_AGENT_TOKEN is required".into()))?;
    let remote = DaemonClient::new(&control_local).with_bearer_token(remote_agent_token);

    // 1. find and track the surface on the remote
    let search_request = SearchRequest {
        query: SearchQuery {
            app_name: args.app_name.clone(),
            title_pattern: args.title_pattern.clone(),
            pids: Vec::new(),
            platform_refs: Vec::new(),
            frontmost: None,
        },
        session: None,
    };
    let search: SearchResponse = with_remote_approval(&args.host, "searching surfaces", || {
        remote.post_json("/surfaces/search", &search_request)
    })
    .await?;
    let candidate = match search.candidates.as_slice() {
        [one] => one.clone(),
        [] => return Err(ClientError::Local(format!("no matching surface on {}", args.host))),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|m| format!("  {} {}", m.app_name.as_deref().unwrap_or("?"), m.title.as_deref().unwrap_or("")))
                .collect();
            return Err(ClientError::Local(format!(
                "{} candidates matched on {}:\n{}",
                many.len(),
                args.host,
                list.join("\n")
            )));
        }
    };
    let tracked: TrackResponse = remote
        .post_json(
            "/surfaces/track",
            &TrackRequest {
                ref_: candidate.ref_.clone(),
                session: None,
            },
        )
        .await?;
    eprintln!(
        "remote surface {} ({} {})",
        tracked.surface_id,
        candidate.app_name.as_deref().unwrap_or("?"),
        candidate.title.as_deref().unwrap_or("")
    );

    // Everything created on the remote from here on is torn down if a later
    // step fails; on success `--hold` decides when it ends.
    let mut remote_state = RemoteState::default();
    let outcome = attach_inner(local, &args, &remote, &tracked, &work, &mut remote_state).await;
    if outcome.is_err() {
        eprintln!("attach failed after remote resources were created; closing them");
        remote_state.close(&remote).await;
    }
    let _ = std::fs::remove_dir_all(&work);
    outcome
}

/// What `attach` has created on the remote so far.
#[derive(Default)]
struct RemoteState {
    session_id: Option<String>,
    export_id: Option<String>,
}

impl RemoteState {
    async fn close(&self, remote: &DaemonClient) {
        if let (Some(session), Some(export)) = (&self.session_id, &self.export_id) {
            if let Err(e) = remote
                .delete_json::<ExportResponse>(&format!("/publications/{session}/exports/{export}"))
                .await
            {
                eprintln!("could not close remote export {export}: {e}");
            }
        }
        if let Some(session) = &self.session_id {
            if let Err(e) = remote.delete_empty(&format!("/capture-sessions/{session}")).await {
                eprintln!("could not close remote capture session {session}: {e}");
            }
        }
    }
}

async fn attach_inner(
    local: &DaemonClient,
    args: &AttachArgs,
    remote: &DaemonClient,
    tracked: &TrackResponse,
    work: &std::path::Path,
    remote_state: &mut RemoteState,
) -> Result<(), ClientError> {
    // 2. native capture session and export on the remote
    let capture_path = format!("/capture-sessions/surfaces/{}?native=true", tracked.surface_id);
    let empty = serde_json::json!({});
    let session: CreateCaptureSessionResponse =
        with_remote_approval(&args.host, "capturing the surface", || remote.post_json(&capture_path, &empty)).await?;
    eprintln!("remote capture session {} ({})", session.session_id, session.status);
    remote_state.session_id = Some(session.session_id.clone());
    // Exporting with input requires Drive on the remote surface, so it can
    // return agent_permission_needed just as the capture did; wait for the
    // operator the same way.
    let export_path = format!("/publications/{}/exports", session.session_id);
    let export_request = CreateExportRequest {
        chroma: args.chroma,
        bitrate_bps: args.bitrate_bps,
        input: args.input,
    };
    let export: ExportResponse = with_remote_approval(&args.host, "exporting the surface", || {
        remote.post_json(&export_path, &export_request)
    })
    .await?;
    eprintln!("remote export {} ({})", export.export_id, export.status);
    remote_state.export_id = Some(export.export_id.clone());

    // 3. forward the export's sockets, then republish locally
    let media_local = work.join("m");
    let control_link_local = work.join("c");
    let _link_forward = Forward::start(
        &args.host,
        &[
            (media_local.clone(), export.media_socket.clone()),
            (control_link_local.clone(), export.control_socket.clone()),
        ],
    )
    .await?;
    let republished = republish(
        local,
        &RepublishRequest {
            media_socket: media_local.to_string_lossy().into_owned(),
            control_socket: control_link_local.to_string_lossy().into_owned(),
            link_token: export.link_token.clone(),
            identities: Identities {
                source: format!("{}:{}", args.host, export.identities.source),
                publication: format!("{}:{}", args.host, export.identities.publication),
            },
            chroma: args.chroma,
            cpu: args.cpu,
            input: args.input,
        },
        args.json,
    )
    .await?;

    if args.hold {
        eprintln!("holding the forwards; interrupt to end the export and republication");
        tokio::signal::ctrl_c()
            .await
            .map_err(|e| ClientError::Local(format!("signal: {e}")))?;
        let _ = close_republication(local, &republished.publication.publication_id).await;
        let _ = export_close(remote, &session.session_id, &export.export_id).await;
        let _ = remote.delete_empty(&format!("/capture-sessions/{}", session.session_id)).await;
    } else {
        eprintln!(
            "the forwards end with this command; rerun with --hold to keep the republication up, or manage the export and republication by id"
        );
    }
    // Reaching here means the remote resources are either closed or deliberately left for manual management.
    *remote_state = RemoteState::default();
    Ok(())
}
