//! Exports and republications: the coordinator role for cross-host bridges.
//!
//! portholed is the graph manager here. An export spawns an egress half
//! (`jackstay-bridge egress --listen`) for a native capture session; the half
//! attaches to portholed's own attach service with the session's token and
//! listens on two Unix sockets under the runtime directory for the peer the
//! consumer side forwards. A republication runs an ingress half as a launchd
//! job with its own Mach service name and reports it as a publication viewers
//! attach to natively. Both are tracked here with the status lines the halves
//! print; nothing about frames passes through the daemon.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(target_os = "macos")]
use jackstay_graph::export::IngressSpec;
use jackstay_graph::{
    ChromaPolicy, Identities,
    export::{BridgeBinary, EgressSpec, HalfHandle, HalfStatus, Phase},
};
use porthole_core::agent_policy::AgentId;
#[cfg(target_os = "macos")]
use porthole_protocol::{capture_sessions::NATIVE_ATTACH_TRANSPORT_MACOS_XPC, publications::PUBLICATION_KIND_REPUBLISHED};
use porthole_protocol::{
    capture_sessions::NativeCaptureInfo,
    publications::{ExportResponse, PublicationResponse, RepublishRequest, RepublishResponse},
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("unknown export {0}")]
    UnknownExport(String),
    #[error("unknown republication {0}")]
    UnknownRepublication(String),
    #[error("only the owner may inspect or end this export")]
    NotOwner,
    #[error("jackstay-bridge executable not found; set JACKSTAY_BRIDGE_BIN or install it beside portholed")]
    BridgeMissing,
    #[error("export runtime directory: {0}")]
    Io(std::io::Error),
    #[error("minting the link token: {0}")]
    Token(std::io::Error),
    #[error("spawning the bridge half: {0}")]
    Spawn(std::io::Error),
    #[error("registry poisoned")]
    Poisoned,
    #[error("republication did not come up: {0}")]
    RepublishFailed(String),
    #[error("republications need launchd on macOS")]
    Unsupported,
    #[error("this daemon has no input pipeline for a remote input channel")]
    InputUnavailable,
}

struct ExportRecord {
    publication_id: String,
    owner: AgentId,
    identities: Identities,
    link_token: String,
    media_socket: PathBuf,
    control_socket: PathBuf,
    handle: HalfHandle,
    /// The input executor bound for this export, when it carries input. Held
    /// so it lives as long as the export and is torn down with it on drop.
    #[cfg(unix)]
    #[allow(dead_code, reason = "kept for its Drop; not read")]
    executor: Option<crate::input_executor::InputExecutor>,
}

#[cfg(target_os = "macos")]
struct RepublishRecord {
    owner: AgentId,
    identities: Identities,
    native: NativeCaptureInfo,
    /// The CPU setup socket the half serves, when one was requested.
    cpu_socket: Option<PathBuf>,
    /// The input socket the half accepts controllers on, when requested.
    input_socket: Option<PathBuf>,
    job: jackstay_graph::export::IngressJob,
}

#[derive(Default)]
struct Inner {
    exports: HashMap<String, ExportRecord>,
    #[cfg(target_os = "macos")]
    republications: HashMap<String, RepublishRecord>,
}

/// The daemon's export and republication table. Cheap to clone.
#[derive(Clone)]
pub struct ExportRegistry {
    inner: Arc<Mutex<Inner>>,
    runtime_dir: Option<PathBuf>,
    /// The input pipeline an export's executor drives when it carries input.
    #[cfg(unix)]
    input: Option<Arc<porthole_core::input_pipeline::InputPipeline>>,
}

impl std::fmt::Debug for ExportRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExportRegistry").field("runtime_dir", &self.runtime_dir).finish()
    }
}

fn phase_name(status: &HalfStatus) -> &'static str {
    match status.phase {
        None | Some(Phase::Starting) => "starting",
        Some(Phase::Listening) => "listening",
        Some(Phase::Running) => "running",
        Some(Phase::Ended) => "ended",
    }
}

fn status_message(status: &HalfStatus) -> Option<String> {
    match (&status.failure, status.exit_code) {
        (Some(f), _) => Some(f.clone()),
        (None, Some(code)) if code != 0 => Some(format!("bridge half exited with status {code}")),
        _ => None,
    }
}

/// Finds the bridge executable: `JACKSTAY_BRIDGE_BIN`, then a sibling of the
/// running daemon (the bundled deployment), then the PATH.
fn locate_bridge() -> Option<BridgeBinary> {
    if let Some(b) = BridgeBinary::locate(None) {
        return Some(b);
    }
    let sibling = std::env::current_exe().ok()?.with_file_name("jackstay-bridge");
    sibling.is_file().then_some(BridgeBinary(sibling))
}

impl ExportRegistry {
    /// `runtime_dir` is where per-export socket directories are created; `None`
    /// disables exports (tests without a socket directory).
    #[must_use]
    pub fn new(runtime_dir: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            runtime_dir,
            #[cfg(unix)]
            input: None,
        }
    }

    /// Sets the input pipeline exports drive when they carry input.
    #[cfg(unix)]
    #[must_use]
    pub fn with_input(mut self, input: Arc<porthole_core::input_pipeline::InputPipeline>) -> Self {
        self.input = Some(input);
        self
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self::new(None)
    }

    fn runtime_dir(&self) -> Result<&Path, ExportError> {
        self.runtime_dir.as_deref().ok_or_else(|| {
            ExportError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "exports are disabled: no runtime directory",
            ))
        })
    }

    /// Spawns an egress half for a native publication owned by `owner`.
    #[allow(clippy::too_many_arguments, reason = "an export is defined by all of these")]
    pub fn create_export(
        &self,
        publication_id: &str,
        owner: AgentId,
        identities: Identities,
        native: &NativeCaptureInfo,
        chroma: ChromaPolicy,
        bitrate_bps: Option<u32>,
        input: bool,
        frame: (u32, u32),
    ) -> Result<ExportResponse, ExportError> {
        let bridge = locate_bridge().ok_or(ExportError::BridgeMissing)?;
        let export_id = format!("exp_{}", Uuid::new_v4().simple());
        // Unix socket paths are limited to 104 bytes on macOS and the runtime
        // directory under the per-user temp dir is already long, so keep the
        // export's directory and socket names short.
        let dir = self.runtime_dir()?.join("x").join(&export_id[4..16]);
        std::fs::create_dir_all(&dir).map_err(ExportError::Io)?;
        // Start the input executor first, if asked, so its socket is on the
        // egress command line.
        #[cfg(unix)]
        let executor = self.start_executor(input, &dir, &identities.source, frame)?;
        #[cfg(unix)]
        let input_socket = executor.as_ref().map(|e| e.path().to_path_buf());
        #[cfg(not(unix))]
        let input_socket = {
            let _ = input;
            None
        };
        let spec = EgressSpec {
            source_service: native.endpoint.clone(),
            source_token: Some(native.attach_token.clone()),
            media_socket: dir.join("m"),
            control_socket: dir.join("c"),
            link_token: jackstay_graph::mint_token().map_err(ExportError::Token)?,
            chroma,
            bitrate_bps,
            input_socket,
        };
        let handle = jackstay_graph::export::spawn_egress(&bridge, &spec).map_err(ExportError::Spawn)?;
        let mut record = ExportRecord {
            publication_id: publication_id.to_owned(),
            owner,
            identities,
            link_token: spec.link_token.clone(),
            media_socket: spec.media_socket.clone(),
            control_socket: spec.control_socket.clone(),
            handle,
            #[cfg(unix)]
            executor,
        };
        // The peer cannot connect before the sockets exist; wait briefly for them.
        let status = record.handle.wait_for(Phase::Listening, Duration::from_secs(5));
        let response = export_response(&export_id, &record, &status);
        self.inner
            .lock()
            .map_err(|_| ExportError::Poisoned)?
            .exports
            .insert(export_id, record);
        Ok(response)
    }

    /// Binds an input executor for an export that carries input, driving the
    /// captured surface. `dir` is the export's socket directory and `surface`
    /// the surface id to inject into.
    #[cfg(unix)]
    fn start_executor(
        &self,
        input: bool,
        dir: &Path,
        surface: &str,
        frame: (u32, u32),
    ) -> Result<Option<crate::input_executor::InputExecutor>, ExportError> {
        if !input {
            return Ok(None);
        }
        let pipeline = self.input.clone().ok_or(ExportError::InputUnavailable)?;
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|e| ExportError::Spawn(std::io::Error::other(format!("no runtime for the input executor: {e}"))))?;
        let executor = crate::input_executor::InputExecutor::start(&dir.join("i"), surface.into(), pipeline, handle, frame.0, frame.1)
            .map_err(ExportError::Spawn)?;
        Ok(Some(executor))
    }

    /// An export is addressed under its publication; one named under another
    /// publication is unknown, not merely misfiled.
    pub fn export_status(&self, publication_id: &str, export_id: &str, agent: &AgentId) -> Result<ExportResponse, ExportError> {
        let mut inner = self.inner.lock().map_err(|_| ExportError::Poisoned)?;
        let record = inner
            .exports
            .get_mut(export_id)
            .filter(|record| record.publication_id == publication_id)
            .ok_or_else(|| ExportError::UnknownExport(export_id.to_owned()))?;
        if &record.owner != agent {
            return Err(ExportError::NotOwner);
        }
        let status = record.handle.status();
        Ok(export_response(export_id, record, &status))
    }

    /// Stops the egress half and forgets the export.
    pub fn close_export(&self, publication_id: &str, export_id: &str, agent: &AgentId) -> Result<ExportResponse, ExportError> {
        let mut inner = self.inner.lock().map_err(|_| ExportError::Poisoned)?;
        let record = inner
            .exports
            .get(export_id)
            .filter(|record| record.publication_id == publication_id)
            .ok_or_else(|| ExportError::UnknownExport(export_id.to_owned()))?;
        if &record.owner != agent {
            return Err(ExportError::NotOwner);
        }
        let mut record = inner.exports.remove(export_id).expect("checked above");
        let status = record.handle.stop();
        let response = export_response(export_id, &record, &status);
        if let Some(dir) = record.media_socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(response)
    }

    /// Export ids for a publication, so closing a publication can end them.
    pub fn exports_for(&self, publication_id: &str) -> Vec<String> {
        self.inner
            .lock()
            .map(|inner| {
                inner
                    .exports
                    .iter()
                    .filter(|(_, r)| r.publication_id == publication_id)
                    .map(|(id, _)| id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Runs an ingress half as a launchd job for the forwarded export
    /// described by `request` and reports the resulting publication.
    #[cfg(target_os = "macos")]
    pub fn republish(&self, owner: AgentId, request: RepublishRequest) -> Result<RepublishResponse, ExportError> {
        let RepublishRequest {
            media_socket,
            control_socket,
            link_token,
            identities,
            chroma,
            cpu,
            input,
        } = request;
        let bridge = locate_bridge().ok_or(ExportError::BridgeMissing)?;
        let publication_id = format!("rep_{}", Uuid::new_v4().simple());
        let service = format!("work.flotilla.porthole.republish.{}", &publication_id[4..20]);
        let dir = self.runtime_dir()?.join("republications").join(&publication_id);
        // Short, like export sockets: Unix socket paths are limited to 104 bytes.
        // The CPU and input sockets share one short directory owned by portholed.
        let sockets_dir = (cpu || input)
            .then(|| self.runtime_dir().map(|r| r.join("r").join(&publication_id[4..16])))
            .transpose()?;
        if let Some(d) = &sockets_dir {
            std::fs::create_dir_all(d).map_err(ExportError::Io)?;
        }
        let cpu_socket = sockets_dir.as_ref().filter(|_| cpu).map(|d| d.join("s"));
        let input_socket = sockets_dir.as_ref().filter(|_| input).map(|d| d.join("i"));
        let spec = IngressSpec {
            media_socket: PathBuf::from(media_socket),
            control_socket: PathBuf::from(control_socket),
            service: service.clone(),
            viewer_token: Some(format!("ptas_{}", Uuid::new_v4().simple())),
            link_token,
            chroma,
            cpu_socket: cpu_socket.clone(),
            input_socket: input_socket.clone(),
        };
        let remove_sockets_dir = || {
            if let Some(d) = &sockets_dir {
                let _ = std::fs::remove_dir_all(d);
            }
        };
        let mut job = jackstay_graph::export::IngressJob::spawn(&bridge, &spec, &dir).map_err(|e| {
            remove_sockets_dir();
            ExportError::RepublishFailed(e.to_string())
        })?;
        let status = job.wait_for_publication(Duration::from_secs(20));
        if status.publication.is_none() {
            let detail = status
                .failure
                .clone()
                .or_else(|| {
                    std::fs::read_to_string(job.stderr_path())
                        .ok()
                        .map(|s| s.lines().last().unwrap_or("").to_owned())
                })
                .unwrap_or_else(|| "no publication within 20 s".to_owned());
            let _ = job.stop();
            remove_sockets_dir();
            return Err(ExportError::RepublishFailed(detail));
        }
        let native = NativeCaptureInfo {
            transport_kind: NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
            endpoint: service,
            attach_token: spec.viewer_token.clone().unwrap_or_default(),
        };
        let record = RepublishRecord {
            owner,
            identities: identities.clone(),
            native: native.clone(),
            // The paths portholed asked for and whose directory it owns; the
            // half reports the same ones, and the directory cleanup keys off
            // this shared parent.
            cpu_socket,
            input_socket,
            job,
        };
        let response = RepublishResponse {
            publication: republish_publication(&publication_id, &record, &status),
            decision: status.decision.clone(),
        };
        self.inner
            .lock()
            .map_err(|_| ExportError::Poisoned)?
            .republications
            .insert(publication_id, record);
        Ok(response)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn republish(&self, _owner: AgentId, _request: RepublishRequest) -> Result<RepublishResponse, ExportError> {
        Err(ExportError::Unsupported)
    }

    /// Republished publications, as the publications view lists them.
    pub fn list_republications(&self) -> Vec<PublicationResponse> {
        #[cfg(target_os = "macos")]
        {
            let mut inner = match self.inner.lock() {
                Ok(i) => i,
                Err(_) => return Vec::new(),
            };
            inner
                .republications
                .iter_mut()
                .map(|(id, record)| {
                    let status = record.job.status();
                    republish_publication(id, record, &status)
                })
                .collect()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Vec::new()
        }
    }

    pub fn republication(&self, publication_id: &str) -> Option<PublicationResponse> {
        #[cfg(target_os = "macos")]
        {
            let mut inner = self.inner.lock().ok()?;
            let record = inner.republications.get_mut(publication_id)?;
            let status = record.job.status();
            Some(republish_publication(publication_id, record, &status))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = publication_id;
            None
        }
    }

    /// Stops the ingress job behind a republication.
    pub fn close_republication(&self, publication_id: &str, agent: &AgentId) -> Result<(), ExportError> {
        #[cfg(target_os = "macos")]
        {
            let mut inner = self.inner.lock().map_err(|_| ExportError::Poisoned)?;
            let record = inner
                .republications
                .get(publication_id)
                .ok_or_else(|| ExportError::UnknownRepublication(publication_id.to_owned()))?;
            if &record.owner != agent {
                return Err(ExportError::NotOwner);
            }
            let mut record = inner.republications.remove(publication_id).expect("checked above");
            let _ = record.job.stop();
            // Both sockets live in one directory; remove it once.
            let dir = record
                .cpu_socket
                .as_ref()
                .or(record.input_socket.as_ref())
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
            if let Some(dir) = dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (publication_id, agent);
            Err(ExportError::UnknownRepublication(publication_id.to_owned()))
        }
    }
}

fn export_response(export_id: &str, record: &ExportRecord, status: &HalfStatus) -> ExportResponse {
    ExportResponse {
        export_id: export_id.to_owned(),
        publication_id: record.publication_id.clone(),
        identities: record.identities.clone(),
        status: phase_name(status).to_owned(),
        status_message: status_message(status),
        decision: status.decision.clone(),
        media_socket: record.media_socket.to_string_lossy().into_owned(),
        control_socket: record.control_socket.to_string_lossy().into_owned(),
        link_token: record.link_token.clone(),
        report: status.report.clone(),
    }
}

#[cfg(target_os = "macos")]
fn republish_publication(publication_id: &str, record: &RepublishRecord, status: &HalfStatus) -> PublicationResponse {
    let (width, height) = status
        .report
        .as_ref()
        .and_then(|r| Some((r.get("width")?.as_u64()? as u32, r.get("height")?.as_u64()? as u32)))
        .unwrap_or((0, 0));
    PublicationResponse {
        publication_id: publication_id.to_owned(),
        kind: PUBLICATION_KIND_REPUBLISHED.to_owned(),
        identities: record.identities.clone(),
        status: match status.phase {
            Some(Phase::Running) => "ready".to_owned(),
            Some(Phase::Ended) => "closed".to_owned(),
            _ => "starting".to_owned(),
        },
        status_message: status_message(status).or_else(|| {
            status
                .decision
                .as_ref()
                .map(|d| format!("{:?} {:?} hardware={} ({})", d.codec, d.chroma, d.hardware, d.reason))
        }),
        width,
        height,
        native: Some(record.native.clone()),
        cpu_socket: record.cpu_socket.as_ref().map(|p| p.to_string_lossy().into_owned()),
        input_socket: record.input_socket.as_ref().map(|p| p.to_string_lossy().into_owned()),
    }
}

#[cfg(test)]
mod tests {
    use porthole_protocol::capture_sessions::NATIVE_ATTACH_TRANSPORT_MACOS_XPC;

    use super::*;

    #[test]
    fn phases_map_to_status_names() {
        let mut s = HalfStatus::default();
        assert_eq!(phase_name(&s), "starting");
        s.phase = Some(Phase::Listening);
        assert_eq!(phase_name(&s), "listening");
        s.phase = Some(Phase::Ended);
        s.exit_code = Some(3);
        assert_eq!(status_message(&s).as_deref(), Some("bridge half exited with status 3"));
    }

    #[test]
    fn disabled_registry_refuses_exports() {
        let registry = ExportRegistry::disabled();
        let native = NativeCaptureInfo {
            transport_kind: NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
            endpoint: "svc".into(),
            attach_token: "t".into(),
        };
        let err = registry
            .create_export(
                "sess",
                AgentId::from("agent_1".to_owned()),
                Identities {
                    source: "s".into(),
                    publication: "sess".into(),
                },
                &native,
                ChromaPolicy::Prefer444,
                None,
                false,
                (0, 0),
            )
            .unwrap_err();
        assert!(matches!(err, ExportError::Io(_) | ExportError::BridgeMissing), "{err}");
    }
}
