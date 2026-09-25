//! Windows native capture sessions (#186), as decided in jackstay#23.
//!
//! Porthole authorizes the caller (the route requires `observe` and `record`
//! on the surface), selects the window through the Windows adapter's verified
//! identity, and creates the window's `GraphicsCaptureItem`
//! (`IGraphicsCaptureItemInterop::CreateForWindow`). It hands the item to
//! Jackstay's `WgcCapture` with host policy: cursor, border acceptability,
//! minimum update interval and output size, plus the arena limits Porthole
//! applies to every native pool. Jackstay owns the D3D11 device, frame pool,
//! copy, fences and adapter identity.
//!
//! Porthole keeps lifecycle policy:
//! - `Closed` (the window went away) fails the session. Nothing restarts it.
//! - Lock and RDP disconnect report the desktop unavailable (`paused`) and
//!   resume with the desktop; they do not fail the session.
//! - Device loss recovers as a new epoch with a new publication; consumers of
//!   the old epoch see it close and attach again.
//!
//! Consumers attach over a per-session Jackstay Local Endpoint (Wheelhouse
//! ADR 0011), `Session` scope, with the session's attach token; see
//! [`NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT`]. One native session at a
//! time, as on macOS and Linux; a closed session holds the slot until its
//! publications drain.

use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use jackstay::{
    acquisition::socket::serve_cpu,
    local::{self, Connection, Endpoint, Listener, Scope, ShutdownHandle, Stream, Transport},
    model::{PixelFormat, SourceDesc, SourceKind, TrackDesc, VideoTrackDesc},
    native::windows::{
        capture::{
            BorderPolicy, CaptureArena, CapturePolicy, CaptureState, CaptureStatus, CaptureTarget, DesktopMonitor, OutputSize, Publication,
            PublicationMode, WgcCapture, capture_item_for_window, is_supported,
        },
        setup::serve_d3d11,
    },
    state::SessionState,
};
use porthole_adapter_windows::WindowsAdapter;
use porthole_core::{ErrorCode, PortholeError, agent_policy::AgentId, surface::SurfaceInfo};
use porthole_protocol::capture_sessions::{
    CaptureBorderPolicy, CaptureOutputPolicy, CaptureSessionRequest, CreateCaptureSessionResponse,
    NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT, NativeCaptureInfo, WindowsNativeAttachReply, WindowsNativeAttachRequest,
    WindowsNativePublication,
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::{
    CAPTURE_MEMORY_BUDGET, CAPTURE_RESOURCE_CAPACITY, CaptureRegistry, CaptureRegistryError, CaptureSession, CaptureSessionLifecycle,
};

const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the session thread observes capture status and polls cleanup.
const WATCH_INTERVAL: Duration = Duration::from_millis(100);
/// A consumer must send its attach line this soon after connecting.
const PREFACE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PREFACE: usize = 16 * 1024;
/// Concurrent setup connections per session (each holds one consumer).
const MAX_CONNECTIONS: usize = 4;
const MAX_MIN_UPDATE_INTERVAL_MS: u32 = 10_000;

/// The arena every Windows native publication uses: Porthole's common native
/// budget and slot count (within D3D11's 15-slot handle batch).
fn capture_arena() -> CaptureArena {
    CaptureArena {
        resource_capacity: CAPTURE_RESOURCE_CAPACITY,
        retained_history: 2,
        producer_reserve: 1,
        memory_budget: CAPTURE_MEMORY_BUDGET,
        max_incarnations: MAX_CONNECTIONS as u32,
        drain_timeout: DRAIN_TIMEOUT,
    }
}

fn invalid(message: impl Into<String>) -> CaptureRegistryError {
    CaptureRegistryError::from_porthole(PortholeError::new(ErrorCode::InvalidArgument, message))
}

/// Map the request's policy onto Jackstay's. `desktop` replaces the session
/// desktop monitor in tests (injected lock and disconnect).
pub(super) fn capture_policy(
    request: &CaptureSessionRequest,
    desktop: Option<Arc<dyn DesktopMonitor>>,
) -> Result<CapturePolicy, CaptureRegistryError> {
    let min_update_interval = match request.min_update_interval_ms {
        None => None,
        Some(ms) if (1..=MAX_MIN_UPDATE_INTERVAL_MS).contains(&ms) => Some(Duration::from_millis(u64::from(ms))),
        Some(_) => {
            return Err(invalid(format!("min_update_interval_ms must be 1..={MAX_MIN_UPDATE_INTERVAL_MS}")));
        }
    };
    let output_size = match request.output {
        None | Some(CaptureOutputPolicy::Source) => OutputSize::Source,
        Some(CaptureOutputPolicy::Fit { width, height }) => {
            super::check_output_budget(width, height)?;
            OutputSize::Fit {
                max_width: width,
                max_height: height,
            }
        }
        Some(CaptureOutputPolicy::Fixed { width, height }) => {
            super::check_output_budget(width, height)?;
            OutputSize::Fixed { width, height }
        }
    };
    Ok(CapturePolicy {
        cursor: request.cursor.unwrap_or(true),
        border: match request.border {
            None | Some(CaptureBorderPolicy::PreferHidden) => BorderPolicy::PreferHidden,
            Some(CaptureBorderPolicy::Show) => BorderPolicy::Show,
            Some(CaptureBorderPolicy::RequireHidden) => BorderPolicy::RequireHidden,
        },
        min_update_interval,
        output_size,
        arena: capture_arena(),
        desktop,
        watch_interval: WATCH_INTERVAL,
        ..CapturePolicy::default()
    })
}

/// Keeps a Windows native session's runtime alive. Dropping it requests the
/// same close as `DELETE`; the session thread owns the capture and drains its
/// publications independently of the registry and the async runtime.
pub(super) struct WindowsNativeSessionHold {
    pub native_info: NativeCaptureInfo,
    runtime: Arc<Runtime>,
    /// The attach endpoint stays bound while the session record exists, so a
    /// consumer arriving after a failure or close is told why. The registry
    /// drops the hold (and so the endpoint) once the session has drained and
    /// a new native session replaces it.
    listener: Arc<Listener>,
}

impl std::fmt::Debug for WindowsNativeSessionHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsNativeSessionHold")
            .field("native_info", &self.native_info)
            .finish_non_exhaustive()
    }
}

impl WindowsNativeSessionHold {
    pub(super) fn close(&mut self) {
        self.runtime.lock().closing.get_or_insert_with(Instant::now);
    }

    pub(super) fn drained(&self) -> bool {
        self.runtime.lock().drained
    }

    pub(super) fn snapshot(&self) -> NativeSnapshot {
        self.runtime.lock().snapshot()
    }
}

impl Drop for WindowsNativeSessionHold {
    fn drop(&mut self) {
        self.close();
        self.listener.cancel();
    }
}

pub(super) struct NativeSnapshot {
    pub status: &'static str,
    pub message: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct Runtime {
    state: Mutex<State>,
}

impl Runtime {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct ConnectionEntry {
    epoch: u64,
    shutdown: ShutdownHandle,
}

#[derive(Default)]
struct State {
    status: Option<CaptureStatus>,
    /// The current epoch's publication.
    publication: Option<(u64, Publication)>,
    /// Earlier epochs' publications (device loss), draining.
    retired: Vec<Publication>,
    connections: BTreeMap<u64, ConnectionEntry>,
    next_connection: u64,
    /// The host closed the session.
    closing: Option<Instant>,
    /// The capture ended on its own: window closed or a capture failure.
    failure: Option<String>,
    /// The capture is stopped and the attach endpoint cancelled.
    stopped: Option<Instant>,
    cleanup_error: Option<String>,
    drained: bool,
    /// Tests: a device loss for the session thread to simulate.
    #[cfg(test)]
    inject_device_loss: Option<String>,
}

/// Run `$body` with the publication's producer locked, whichever arena it is.
macro_rules! with_producer {
    ($publication:expr, |$producer:ident| $body:expr) => {
        match $publication {
            Publication::D3d11(shared) => {
                let mut $producer = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                $body
            }
            Publication::Cpu(shared) => {
                let mut $producer = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                $body
            }
        }
    };
}

fn publication_owners(publication: &Publication) -> usize {
    match publication {
        Publication::D3d11(shared) => Arc::strong_count(shared),
        Publication::Cpu(shared) => Arc::strong_count(shared),
    }
}

/// Reclaim dead consumers; when stopped, report whether the publication can
/// be released (its consumers' mappings and GPU use retired).
fn poll_publication(publication: &Publication, stopped: bool) -> (Option<String>, bool) {
    with_producer!(publication, |producer| {
        let mut error = match producer.poll_cleanup() {
            Err(error) => Some(error.to_string()),
            Ok(_) => {
                let failures = producer.cleanup_failures();
                (!failures.is_empty()).then(|| {
                    failures
                        .into_iter()
                        .map(|failure| format!("{:?}: {}", failure.incarnation, failure.reason))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
            }
        };
        let ready = stopped
            && match producer.poll_shutdown_ready() {
                Ok(ready) => ready,
                Err(failure) => {
                    error.get_or_insert(failure.to_string());
                    false
                }
            };
        (error, ready)
    })
}

impl State {
    /// Record a capture status and the capture's current publication.
    fn observe(&mut self, status: CaptureStatus, publication: Option<(u64, Publication)>) {
        if let Some((epoch, publication)) = publication
            && self.publication.as_ref().map(|(current, _)| *current) != Some(epoch)
        {
            if let Some((old, previous)) = self.publication.replace((epoch, publication)) {
                // Device loss: the old publication is stopped. Close its
                // setup connections so those consumers see the producer go
                // and attach again, and so their owners drop.
                for entry in self.connections.values().filter(|entry| entry.epoch == old) {
                    entry.shutdown.shutdown();
                }
                self.retired.push(previous);
            }
        }
        match &status.state {
            CaptureState::Closed if self.closing.is_none() => {
                let detail = if status.notes.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", status.notes.join("; "))
                };
                self.failure.get_or_insert(format!("captured window closed{detail}"));
            }
            CaptureState::Failed(message) if self.closing.is_none() => {
                self.failure.get_or_insert(format!("capture failed: {message}"));
            }
            _ => {}
        }
        self.status = Some(status);
    }

    fn should_stop(&self) -> bool {
        self.closing.is_some() || self.failure.is_some()
    }

    /// After the capture stopped: stop every publication and close every
    /// setup connection, so consumers see closure and their owners drop.
    fn stop_publications(&mut self) {
        self.stopped.get_or_insert_with(Instant::now);
        for publication in self.publication.iter().map(|(_, publication)| publication).chain(&self.retired) {
            with_producer!(publication, |producer| producer.stop());
        }
        for entry in self.connections.values() {
            entry.shutdown.shutdown();
        }
    }

    fn maintain(&mut self) {
        let stopped = self.stopped.is_some();
        let mut cleanup_error = None;
        self.retired.retain(|publication| {
            // A retired publication is already stopped by the capture.
            let (error, ready) = poll_publication(publication, true);
            if error.is_some() {
                cleanup_error = error;
            }
            !(ready && publication_owners(publication) == 1)
        });
        if let Some((_, publication)) = &self.publication {
            let (error, ready) = poll_publication(publication, stopped);
            if error.is_some() {
                cleanup_error = error;
            }
            if ready && publication_owners(publication) == 1 && self.connections.is_empty() && self.retired.is_empty() {
                self.publication = None;
            }
        }
        self.cleanup_error = cleanup_error;
        if let Some(started) = self.stopped {
            if self.publication.is_none() && self.retired.is_empty() && self.connections.is_empty() {
                self.drained = true;
                self.cleanup_error = None;
            } else if started.elapsed() >= DRAIN_TIMEOUT && self.cleanup_error.is_none() {
                self.cleanup_error =
                    Some("native shutdown needs recovery: consumer mappings, GPU use or setup connections have not retired".to_owned());
            }
        }
    }

    fn open_connection(&mut self, shutdown: ShutdownHandle) -> Result<(u64, u64, Publication), String> {
        if self.should_stop() || self.stopped.is_some() {
            return Err(self
                .failure
                .clone()
                .unwrap_or_else(|| "native capture session is closed".to_owned()));
        }
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err("native capture setup connection limit reached".to_owned());
        }
        let (epoch, publication) = self
            .publication
            .clone()
            .ok_or_else(|| "native capture has no publication".to_owned())?;
        let id = self.next_connection + 1;
        self.next_connection = id;
        self.connections.insert(id, ConnectionEntry { epoch, shutdown });
        Ok((id, epoch, publication))
    }

    fn snapshot(&self) -> NativeSnapshot {
        let status = self.status.as_ref();
        let (name, detail) = if self.drained {
            match &self.failure {
                Some(failure) => ("failed", format!("{failure}; native resources retired")),
                None => ("closed", "native resources retired".to_owned()),
            }
        } else if let Some(error) = &self.cleanup_error {
            ("recovery_required", error.clone())
        } else if let Some(failure) = &self.failure {
            ("failed", failure.clone())
        } else if self.closing.is_some() {
            ("draining", "waiting for consumer mappings and GPU use to retire".to_owned())
        } else {
            match status.map(|status| &status.state) {
                None => ("starting", String::new()),
                Some(CaptureState::Running) => ("ready", String::new()),
                Some(CaptureState::Paused(reason)) => ("paused", format!("desktop unavailable: {reason}")),
                Some(CaptureState::Recovering { reason }) => ("recovering", format!("device lost: {reason}")),
                Some(state) => ("draining", format!("capture {state:?}")),
            }
        };
        let counters = status.map_or_else(String::new, |status| {
            let mode = match &status.mode {
                PublicationMode::D3d11 { adapter } => format!("d3d11 on adapter {} ({})", adapter.luid, adapter.description),
                PublicationMode::Cpu { adapter, reason } => {
                    format!("cpu on adapter {} ({}): {reason}", adapter.luid, adapter.description)
                }
            };
            let border = match status.border_shown {
                Some(true) => "shown",
                Some(false) => "hidden",
                None => "unknown",
            };
            let notes = if status.notes.is_empty() {
                String::new()
            } else {
                format!(", notes: {}", status.notes.join("; "))
            };
            format!(
                "epoch={}, publication={mode}, published={}, dropped={}, border={border}{notes}",
                status.epoch, status.frames_published, status.frames_dropped
            )
        });
        let message = [detail, counters]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        NativeSnapshot {
            status: name,
            message: (!message.is_empty()).then_some(message),
            width: status.map_or(0, |status| status.size.0),
            height: status.map_or(0, |status| status.size.1),
        }
    }
}

/// The session thread: owns the capture, follows its status, stops it on
/// close or terminal failure, then drains every publication.
fn run_session(runtime: Arc<Runtime>, capture: WgcCapture) {
    let mut capture = Some(capture);
    let mut revision = 0;
    loop {
        if let Some(active) = capture.as_ref() {
            #[cfg(test)]
            if let Some(reason) = runtime.lock().inject_device_loss.take() {
                active.simulate_device_loss(&reason);
            }
            let status = active.wait_for_change(revision, WATCH_INTERVAL);
            revision = status.revision;
            let publication = active.publication();
            let stop = {
                let mut state = runtime.lock();
                state.observe(status, publication);
                state.should_stop()
            };
            if stop {
                let mut active = capture.take().expect("capture is active");
                active.stop();
                let last = active.status();
                // Releases the capture's own publication owners.
                drop(active);
                let mut state = runtime.lock();
                state.observe(last, None);
                state.stop_publications();
            }
        } else {
            std::thread::sleep(WATCH_INTERVAL);
        }
        let mut state = runtime.lock();
        state.maintain();
        if state.drained {
            break;
        }
    }
}

fn accept_loop(runtime: Arc<Runtime>, listener: Arc<Listener>, session_id: String, attach_token: String) {
    loop {
        match listener.accept() {
            Ok(connection) => {
                let runtime = Arc::clone(&runtime);
                let session_id = session_id.clone();
                let attach_token = attach_token.clone();
                let spawned = std::thread::Builder::new().name("windows-native-attach".to_owned()).spawn(move || {
                    let peer = connection.peer().clone();
                    if let Err(error) = serve_connection(&runtime, connection, &session_id, &attach_token) {
                        tracing::debug!(session_id, pid = peer.pid, error, "native capture attach connection ended");
                    }
                });
                if let Err(error) = spawned {
                    tracing::warn!(%error, "cannot start a native capture attach thread");
                }
            }
            Err(local::Error::Cancelled) => break,
            Err(local::Error::RefusedPeer(message)) => {
                tracing::warn!(session_id, message, "refused a native capture attach peer");
            }
            Err(error) => {
                tracing::warn!(session_id, %error, "native capture attach accept failed");
                std::thread::sleep(WATCH_INTERVAL);
            }
        }
    }
}

/// Removes a setup connection from the session when its serve ends.
struct ConnectionGuard<'a> {
    runtime: &'a Runtime,
    id: u64,
}

impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        self.runtime.lock().connections.remove(&self.id);
    }
}

fn read_line(stream: &mut Stream) -> std::io::Result<Vec<u8>> {
    // One byte at a time: nothing after the newline belongs to this preface.
    let mut bytes = Vec::new();
    loop {
        if bytes.len() == MAX_PREFACE {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "attach preface is too large"));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        if byte == *b"\n" {
            return Ok(bytes);
        }
        bytes.push(byte[0]);
    }
}

fn write_reply(stream: &mut Stream, reply: &WindowsNativeAttachReply) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(reply).map_err(std::io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()
}

/// Check one attach request against this session. The token is the session's
/// capability; the endpoint's DACL and session check already limit peers to
/// this user's logon session.
fn authorize(request: &[u8], session_id: &str, attach_token: &str) -> Result<(), String> {
    let WindowsNativeAttachRequest::OpenNativeCapture {
        session_id: requested,
        attach_token: presented,
    } = serde_json::from_slice(request).map_err(|error| format!("invalid attach request: {error}"))?;
    let token_matches: bool = presented.as_bytes().ct_eq(attach_token.as_bytes()).into();
    if requested != session_id || !token_matches {
        return Err("attach request is not authorized for this capture session".to_owned());
    }
    Ok(())
}

fn serve_connection(runtime: &Runtime, connection: Connection, session_id: &str, attach_token: &str) -> Result<(), String> {
    let mut stream = connection.into_stream();
    stream.set_read_timeout(Some(PREFACE_TIMEOUT)).map_err(|error| error.to_string())?;
    let request = read_line(&mut stream).map_err(|error| error.to_string())?;
    let opened = authorize(&request, session_id, attach_token).and_then(|()| {
        let shutdown = local::shutdown_handle(&stream).map_err(|error| error.to_string())?;
        runtime.lock().open_connection(shutdown)
    });
    let (id, epoch, publication) = match opened {
        Ok(opened) => opened,
        Err(message) => {
            let _ = write_reply(&mut stream, &WindowsNativeAttachReply::Rejected { message: message.clone() });
            return Err(message);
        }
    };
    let _guard = ConnectionGuard { runtime, id };
    write_reply(
        &mut stream,
        &WindowsNativeAttachReply::NativeCaptureOpened {
            publication: match publication {
                Publication::D3d11(_) => WindowsNativePublication::D3d11,
                Publication::Cpu(_) => WindowsNativePublication::Cpu,
            },
            epoch,
        },
    )
    .map_err(|error| error.to_string())?;
    stream.set_read_timeout(None).map_err(|error| error.to_string())?;
    match publication {
        Publication::D3d11(producer) => serve_d3d11(stream, producer).map_err(|error| error.to_string()),
        Publication::Cpu(producer) => serve_cpu(stream, producer).map_err(|error| error.to_string()),
    }
}

/// Holds the `native_session_starting` reservation across the async startup
/// and removes a half-created session record, unless the session committed.
struct StartReservation<'a> {
    registry: &'a CaptureRegistry,
    active: bool,
    session_id: Option<String>,
}

impl Drop for StartReservation<'_> {
    fn drop(&mut self) {
        if self.active
            && let Ok(mut inner) = self.registry.inner.lock()
        {
            if let Some(id) = &self.session_id {
                inner.sessions.remove(id);
            }
            inner.native_session_starting = false;
        }
    }
}

/// Resolve the window, create its capture item, start Jackstay's capture and
/// wait for its first published frame. Blocking; runs off the async runtime.
fn start_capture(adapter: &WindowsAdapter, surface: &SurfaceInfo, policy: CapturePolicy) -> Result<WgcCapture, CaptureRegistryError> {
    use windows::Win32::Foundation::HWND;
    if !is_supported() {
        return Err(CaptureRegistryError::from_porthole(PortholeError::new(
            ErrorCode::AdapterUnsupported,
            "Windows.Graphics.Capture is not available on this system",
        )));
    }
    let window = adapter
        .native_capture_window(surface)
        .map_err(CaptureRegistryError::from_porthole)?;
    let item = capture_item_for_window(HWND(window as *mut std::ffi::c_void))
        .map_err(|error| CaptureRegistryError::Capture(format!("create the window's capture item: {error}")))?;
    // The item names whatever window the HWND named when it was created. If
    // the identity still verifies, that was the authorized window.
    if adapter
        .native_capture_window(surface)
        .map_err(CaptureRegistryError::from_porthole)?
        != window
    {
        return Err(CaptureRegistryError::from_porthole(PortholeError::new(
            ErrorCode::SurfaceDead,
            "tracked window changed while its capture item was created",
        )));
    }
    let capture = WgcCapture::start(
        CaptureTarget::window_client_area(item, HWND(window as *mut std::ffi::c_void)),
        policy,
    )
    .map_err(|error| CaptureRegistryError::Capture(format!("start Windows.Graphics.Capture: {error}")))?;
    let deadline = Instant::now() + FIRST_FRAME_TIMEOUT;
    let mut status = capture.status();
    while status.frames_published == 0 {
        if status.state.is_terminal() {
            return Err(CaptureRegistryError::Capture(format!(
                "native capture ended before its first frame: {:?}",
                status.state
            )));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(CaptureRegistryError::Capture(format!(
                "native capture produced no first frame ({:?})",
                status.state
            )));
        }
        status = capture.wait_for_change(status.revision, remaining);
    }
    Ok(capture)
}

pub(super) async fn create(
    registry: &CaptureRegistry,
    adapter: Arc<WindowsAdapter>,
    surface: SurfaceInfo,
    owner_agent_id: AgentId,
    request: &CaptureSessionRequest,
    desktop: Option<Arc<dyn DesktopMonitor>>,
) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
    let policy = capture_policy(request, desktop)?;
    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        if inner.native_session_starting {
            return Err(CaptureRegistryError::Capture("a native capture session is starting".to_owned()));
        }
        let retired: Vec<_> = inner
            .native_holds
            .iter()
            .filter(|(_, hold)| hold.drained())
            .map(|(id, _)| id.clone())
            .collect();
        for id in retired {
            inner.native_holds.remove(&id);
            inner.sessions.remove(&id);
        }
        if let Some((id, hold)) = inner.native_holds.iter().next() {
            let snapshot = hold.snapshot();
            return Err(CaptureRegistryError::Capture(format!(
                "a native capture session is active (one at a time): {id} is {}{}",
                snapshot.status,
                snapshot.message.map(|message| format!(": {message}")).unwrap_or_default()
            )));
        }
        inner.native_session_starting = true;
    }
    let mut reservation = StartReservation {
        registry,
        active: true,
        session_id: None,
    };

    let session_id = Uuid::new_v4().to_string();
    let mut state = SessionState::new();
    let source_id = state
        .register_source(SourceDesc {
            kind: SourceKind::Window,
            label: surface.title.clone().unwrap_or_else(|| surface.id.to_string()),
        })
        .map_err(CaptureRegistryError::from_capture)?;
    let track_id = state
        .register_track(
            source_id,
            TrackDesc::Video(VideoTrackDesc {
                width: 0,
                height: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
            }),
        )
        .map_err(CaptureRegistryError::from_capture)?;
    registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?.sessions.insert(
        session_id.clone(),
        CaptureSession {
            source_id,
            track_id,
            owner_agent_id: Some(owner_agent_id),
            surface_id: Some(surface.id.to_string()),
            lifecycle: CaptureSessionLifecycle::Starting,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: PixelFormat::Bgra8Unorm,
            capture_task: None,
            startup_cancel: None,
            output_control: None,
        },
    );
    reservation.session_id = Some(session_id.clone());

    // A cancelled request detaches this task; the capture it returns is then
    // dropped, which stops it.
    let capture = tokio::task::spawn_blocking(move || start_capture(&adapter, &surface, policy))
        .await
        .map_err(|error| CaptureRegistryError::Capture(format!("native capture startup task: {error}")))??;

    let endpoint_name = format!("porthole.capture.{}", Uuid::new_v4().simple());
    let endpoint = Endpoint::new(Scope::Session, &endpoint_name, Transport::LocalStream)
        .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    let listener =
        Arc::new(Listener::bind(&endpoint).map_err(|error| CaptureRegistryError::Io(format!("bind native attach endpoint: {error}")))?);
    let attach_token = format!("ptas_{}", Uuid::new_v4().simple());
    let native_info = NativeCaptureInfo {
        transport_kind: NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT,
        endpoint: endpoint_name,
        attach_token: attach_token.clone(),
    };

    let runtime = Arc::new(Runtime::default());
    let status = capture.status();
    let (width, height) = status.size;
    runtime.lock().observe(status, capture.publication());
    {
        let runtime = Arc::clone(&runtime);
        std::thread::Builder::new()
            .name("windows-native-capture".to_owned())
            .spawn(move || run_session(runtime, capture))
            .map_err(|error| CaptureRegistryError::Capture(format!("start native capture session thread: {error}")))?;
    }
    let hold = WindowsNativeSessionHold {
        native_info: native_info.clone(),
        runtime: Arc::clone(&runtime),
        listener: Arc::clone(&listener),
    };
    {
        let runtime = Arc::clone(&runtime);
        let listener = Arc::clone(&listener);
        let session_id = session_id.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("windows-native-accept".to_owned())
            .spawn(move || accept_loop(runtime, listener, session_id, attach_token))
        {
            drop(hold);
            return Err(CaptureRegistryError::Capture(format!("start native attach thread: {error}")));
        }
    }

    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let Some(session) = inner.sessions.get_mut(&session_id) else {
            return Err(CaptureRegistryError::Closed {
                session_id,
                message: "native capture session closed during startup".to_owned(),
            });
        };
        session.lifecycle = CaptureSessionLifecycle::Ready;
        session.width = width;
        session.height = height;
        session.stride = width.saturating_mul(4);
        inner.native_holds.insert(session_id.clone(), hold);
        inner.native_session_starting = false;
    }
    reservation.active = false;

    Ok(CreateCaptureSessionResponse {
        session_id,
        source_id: source_id.get(),
        track_id: track_id.get(),
        status: CaptureSessionLifecycle::Ready.status_name().to_string(),
        status_message: CaptureSessionLifecycle::Ready.status_message(),
        fd_socket_path: String::new(),
        native: Some(native_info),
    })
}

#[cfg(test)]
mod tests;
