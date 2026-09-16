#![cfg_attr(windows, allow(dead_code))]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[cfg(unix)]
use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    thread,
};

use jackstay::model::{ClockDomain, ColorSpace, DamageKind, PixelFormat, SourceId, TrackId};
#[cfg(unix)]
use jackstay::{
    acquisition::arena::{FrameDescriptor, PublishOutcome},
    model::{FrameSyncKind, PayloadKind, SourceDesc, SourceKind, TrackDesc, VideoTrackDesc},
    state::SessionState,
};
#[cfg(unix)]
use porthole_core::adapter::{
    VideoCaptureFrame, VideoCaptureFrameMetadata, VideoCaptureFramePublisher, VideoCaptureFrameView, VideoCaptureSession,
};
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{
        Adapter, VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureOutputControl, VideoCaptureOutputSize,
        VideoCapturePixelFormat, VideoCaptureTimestampClock,
    },
    agent_policy::AgentId,
    surface::SurfaceInfo,
};
use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};
#[cfg(unix)]
use uuid::Uuid;

use crate::agent_store::AgentPolicyStore;

#[cfg(unix)]
mod cpu_session;

// Shared host policy for CPU/macOS native pools and output-request preflight.
const CAPTURE_MEMORY_BUDGET: u64 = 512 * 1024 * 1024;
const CAPTURE_RESOURCE_CAPACITY: u32 = 8;
#[cfg(unix)]
const RETIRED_CPU_SESSION_LIMIT: usize = 64;

/// An HTTP create future can be dropped at either adapter startup or the first
/// frame await. Keep teardown armed until ownership is handed to the caller.
#[cfg(unix)]
struct CpuStartup {
    registry: CaptureRegistry,
    session_id: Option<String>,
}

#[cfg(unix)]
impl Drop for CpuStartup {
    fn drop(&mut self) {
        if let Some(id) = &self.session_id
            && let Ok(mut inner) = self.registry.inner.lock()
            && let Some(session) = inner.sessions.get_mut(id)
        {
            session.stop_capture();
            if matches!(
                session.lifecycle,
                CaptureSessionLifecycle::Starting | CaptureSessionLifecycle::Ready
            ) {
                session.lifecycle = CaptureSessionLifecycle::Closed("capture startup cancelled".to_owned());
            }
        }
    }
}
#[cfg(target_os = "macos")]
mod native_session;
#[cfg(target_os = "linux")]
mod native_session_linux;

#[derive(Clone)]
pub struct CaptureRegistry {
    inner: Arc<Mutex<CaptureRegistryInner>>,
    fd_socket_path: Option<PathBuf>,
    agent_store: Option<AgentPolicyStore>,
}

impl std::fmt::Debug for CaptureRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureRegistry")
            .field("inner", &self.inner)
            .field("fd_socket_path", &self.fd_socket_path)
            .field("has_agent_store", &self.agent_store.is_some())
            .finish()
    }
}

#[derive(Debug, Default)]
struct CaptureRegistryInner {
    sessions: HashMap<String, CaptureSession>,
    /// Bounded status history, ordered by actual CPU retirement completion.
    #[cfg(unix)]
    retired_cpu_sessions: std::collections::VecDeque<String>,
    /// Runtime owners for native sessions, including closing macOS sessions
    /// whose consumer mappings or GPU work have not yet drained.
    #[cfg(target_os = "macos")]
    native_holds: HashMap<String, native_session::NativeSessionHold>,
    #[cfg(target_os = "linux")]
    native_holds: HashMap<String, native_session_linux::LinuxNativeSessionHold>,
    /// Reserved across native startup, including its cancellable awaits.
    /// Collapses the one-session check-and-reserve into a single locked step
    /// so two concurrent native creates can't both pass the limit.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    native_session_starting: bool,
    #[cfg(test)]
    test_native_info: HashMap<String, porthole_protocol::capture_sessions::NativeCaptureInfo>,
}

#[derive(Debug)]
struct CaptureSession {
    source_id: SourceId,
    track_id: TrackId,
    owner_agent_id: Option<AgentId>,
    /// The tracked surface this session captures; `None` for synthetic sessions.
    surface_id: Option<String>,
    lifecycle: CaptureSessionLifecycle,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
    #[cfg(unix)]
    cpu: Option<cpu_session::CpuSession>,
    capture_task: Option<tokio::task::JoinHandle<()>>,
    startup_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    output_control: Option<Arc<dyn VideoCaptureOutputControl>>,
}

impl CaptureSession {
    fn stop_capture(&mut self) {
        #[cfg(unix)]
        if let Some(cpu) = &self.cpu {
            cpu.stop();
        }
        if let Some(task) = self.capture_task.take() {
            task.abort();
        }
        if let Some(cancel) = self.startup_cancel.take() {
            let _ = cancel.send(());
        }
    }
}
impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.stop_capture();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CaptureSessionLifecycle {
    Starting,
    Ready,
    Closed(String),
    Failed(String),
}

impl CaptureSessionLifecycle {
    const fn status_name(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Closed(_) => "closed",
            Self::Failed(_) => "failed",
        }
    }

    fn status_message(&self) -> Option<String> {
        match self {
            Self::Closed(message) | Self::Failed(message) => Some(message.clone()),
            Self::Starting | Self::Ready => None,
        }
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FirstFrameInfo {
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
}

#[cfg(unix)]
#[derive(Debug)]
struct RegistryVideoFramePublisher {
    registry: CaptureRegistry,
    session_id: String,
    first_frame_tx: Mutex<Option<tokio::sync::oneshot::Sender<FirstFrameInfo>>>,
}

#[cfg(unix)]
impl RegistryVideoFramePublisher {
    fn new(registry: CaptureRegistry, session_id: String, first_frame_tx: tokio::sync::oneshot::Sender<FirstFrameInfo>) -> Self {
        Self {
            registry,
            session_id,
            first_frame_tx: Mutex::new(Some(first_frame_tx)),
        }
    }
}

#[cfg(unix)]
impl VideoCaptureFramePublisher for RegistryVideoFramePublisher {
    fn publish_frame(&self, frame: VideoCaptureFrameView<'_>) -> Result<(), PortholeError> {
        let mut inner = self
            .registry
            .inner
            .lock()
            .map_err(|_| PortholeError::new(ErrorCode::InternalError, "capture registry lock poisoned"))?;
        let session = inner
            .sessions
            .get_mut(&self.session_id)
            .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, format!("unknown capture session {}", self.session_id)))?;
        session.width = frame.metadata.width;
        session.height = frame.metadata.height;
        session.stride = frame.metadata.stride;
        session.pixel_format = capture_pixel_format(frame.metadata.pixel_format);
        if !matches!(
            session.lifecycle,
            CaptureSessionLifecycle::Starting | CaptureSessionLifecycle::Ready
        ) {
            return Err(PortholeError::new(ErrorCode::InternalError, "capture session is terminal"));
        }
        let outcome = publish_capture_frame_view(session.cpu.as_ref().expect("CPU session"), frame)
            .map_err(|error| PortholeError::new(ErrorCode::InternalError, error.to_string()))?;
        if outcome == PublishOutcome::Dropped {
            return Ok(());
        }
        if !matches!(session.lifecycle, CaptureSessionLifecycle::Ready) {
            session.lifecycle = CaptureSessionLifecycle::Ready;
        }
        if let Some(tx) = self
            .first_frame_tx
            .lock()
            .map_err(|_| PortholeError::new(ErrorCode::InternalError, "capture first-frame lock poisoned"))?
            .take()
        {
            let _ = tx.send(FirstFrameInfo {
                width: session.width,
                height: session.height,
                stride: session.stride,
                pixel_format: session.pixel_format,
            });
        }
        Ok(())
    }
}

impl CaptureRegistry {
    #[cfg(unix)]
    fn new_cpu_session(&self, session_id: &str) -> Result<cpu_session::CpuSession, CaptureRegistryError> {
        let registry = Arc::downgrade(&self.inner);
        let session_id = session_id.to_owned();
        cpu_session::CpuSession::with_retirement_callback(move || {
            // The CPU worker has released its runtime lock before notifying us.
            // A weak reference avoids keeping the registry alive during drain.
            if let Some(registry) = registry.upgrade()
                && let Ok(mut inner) = registry.lock()
                && inner.sessions.contains_key(&session_id)
            {
                inner.retired_cpu_sessions.push_back(session_id);
                while inner.retired_cpu_sessions.len() > RETIRED_CPU_SESSION_LIMIT {
                    let id = inner.retired_cpu_sessions.pop_front().expect("retired history is nonempty");
                    inner.sessions.remove(&id);
                }
            }
        })
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CaptureRegistryInner::default())),
            fd_socket_path: None,
            agent_store: None,
        }
    }

    #[cfg(not(unix))]
    pub fn create_synthetic_session(&self) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        Err(CaptureRegistryError::FdSocketDisabled)
    }

    #[cfg(not(unix))]
    pub async fn create_surface_session(
        &self,
        _adapter: Arc<dyn Adapter>,
        _surface: SurfaceInfo,
        _owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        Err(CaptureRegistryError::FdSocketDisabled)
    }

    #[must_use]
    pub fn disabled_with_agent_policy(agent_store: AgentPolicyStore) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CaptureRegistryInner::default())),
            fd_socket_path: None,
            agent_store: Some(agent_store),
        }
    }

    pub fn with_fd_socket(fd_socket_path: PathBuf) -> std::io::Result<Self> {
        Self::with_optional_fd_socket_agent_policy(fd_socket_path, None)
    }

    pub fn with_fd_socket_and_agent_policy(fd_socket_path: PathBuf, agent_store: AgentPolicyStore) -> std::io::Result<Self> {
        Self::with_optional_fd_socket_agent_policy(fd_socket_path, Some(agent_store))
    }

    #[cfg(unix)]
    fn with_optional_fd_socket_agent_policy(fd_socket_path: PathBuf, agent_store: Option<AgentPolicyStore>) -> std::io::Result<Self> {
        if let Some(parent) = fd_socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if fd_socket_path.exists() {
            std::fs::remove_file(&fd_socket_path)?;
        }
        let listener = UnixListener::bind(&fd_socket_path)?;
        let registry = Self {
            inner: Arc::new(Mutex::new(CaptureRegistryInner::default())),
            fd_socket_path: Some(fd_socket_path),
            agent_store,
        };
        spawn_fd_listener(listener, registry.clone());
        Ok(registry)
    }

    #[cfg(windows)]
    fn with_optional_fd_socket_agent_policy(_fd_socket_path: PathBuf, agent_store: Option<AgentPolicyStore>) -> std::io::Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(CaptureRegistryInner::default())),
            fd_socket_path: None,
            agent_store,
        })
    }

    #[cfg(unix)]
    pub fn create_synthetic_session(&self) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        let fd_socket_path = self.fd_socket_path()?;
        let session_id = Uuid::new_v4().to_string();
        let mut state = SessionState::new();
        let source_id = state
            .register_source(SourceDesc {
                kind: SourceKind::Window,
                label: "synthetic".to_string(),
            })
            .map_err(CaptureRegistryError::from_capture)?;
        let track_id = state
            .register_track(
                source_id,
                TrackDesc::Video(VideoTrackDesc {
                    width: 2,
                    height: 1,
                    pixel_format: PixelFormat::Bgra8Unorm,
                }),
            )
            .map_err(CaptureRegistryError::from_capture)?;
        // SessionState gives us typed id allocation and validation here; the
        // current capture-session registry stores the resulting ids directly
        // until event replay is exposed over the daemon boundary.
        // TODO: retain or replay these events once daemon consumers subscribe
        // to generic session setup instead of synthesizing attach events.

        let cpu = self.new_cpu_session(&session_id)?;
        cpu.publish(
            FrameDescriptor {
                sequence: 1,
                width: 2,
                height: 1,
                stride: 8,
                pixel_format: PixelFormat::Bgra8Unorm as u32,
                sync_kind: FrameSyncKind::CpuCopyComplete as u32,
                damage_kind: DamageKind::FullFrame as u32,
                damage_base_sequence: 1,
                payload_kind: PayloadKind::CpuShm as u32,
                ..FrameDescriptor::default()
            },
            &[0, 64, 128, 255, 255, 64, 128, 255],
        )?;

        let session = CaptureSession {
            source_id,
            track_id,
            owner_agent_id: None,
            surface_id: None,
            lifecycle: CaptureSessionLifecycle::Ready,
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu),
            capture_task: None,
            startup_cancel: None,
            output_control: None,
        };
        self.inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .insert(session_id.clone(), session);

        Ok(CreateCaptureSessionResponse {
            session_id,
            source_id: source_id.get(),
            track_id: track_id.get(),
            status: CaptureSessionLifecycle::Ready.status_name().to_string(),
            status_message: CaptureSessionLifecycle::Ready.status_message(),
            fd_socket_path,
            native: None,
        })
    }

    #[cfg(unix)]
    pub async fn create_surface_session(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
        owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        match self
            .create_surface_session_with_publisher(adapter.clone(), surface.clone(), owner_agent_id.clone())
            .await
        {
            Ok(response) => return Ok(response),
            Err(CaptureRegistryError::Porthole(error)) if error.code == ErrorCode::AdapterUnsupported => {}
            Err(error) => return Err(error),
        }

        self.create_surface_session_with_owned_frames(adapter, surface, owner_agent_id)
            .await
    }

    #[cfg(unix)]
    async fn create_surface_session_with_publisher(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
        owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        let fd_socket_path = self.fd_socket_path()?;
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

        let (startup_cancel_tx, startup_cancel_rx) = tokio::sync::oneshot::channel();
        self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?.sessions.insert(
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
                cpu: Some(self.new_cpu_session(&session_id)?),
                capture_task: None,
                startup_cancel: Some(startup_cancel_tx),
                output_control: None,
            },
        );

        let (first_frame_tx, first_frame_rx) = tokio::sync::oneshot::channel();
        let mut startup = CpuStartup {
            registry: self.clone(),
            session_id: Some(session_id.clone()),
        };
        let publisher = Arc::new(RegistryVideoFramePublisher::new(self.clone(), session_id.clone(), first_frame_tx));
        let capture = match adapter.start_video_capture_publisher(&surface, publisher).await {
            Ok(capture) => capture,
            Err(error) => {
                self.remove_session(&session_id);
                return Err(CaptureRegistryError::from_porthole(error));
            }
        };
        let output_control = capture.output_control();
        let task = tokio::spawn(run_capture_session_monitor(self.clone(), session_id.clone(), capture));
        {
            let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
            let Some(session) = inner.sessions.get_mut(&session_id) else {
                task.abort();
                return Err(CaptureRegistryError::Closed {
                    session_id,
                    message: "capture session closed during startup".to_string(),
                });
            };
            if !matches!(
                session.lifecycle,
                CaptureSessionLifecycle::Starting | CaptureSessionLifecycle::Ready
            ) {
                task.abort();
                return Err(CaptureRegistryError::Closed {
                    session_id,
                    message: "capture session closed during startup".to_string(),
                });
            }
            session.capture_task = Some(task);
            session.output_control = output_control;
        }

        let first_frame = tokio::select! {
            result = tokio::time::timeout(std::time::Duration::from_secs(5), first_frame_rx) => match result {
                Ok(Ok(first_frame)) => first_frame,
                Ok(Err(_)) => {
                    self.remove_session(&session_id);
                    return Err(CaptureRegistryError::Capture(
                        "capture publisher ended before first frame".to_string(),
                    ));
                }
                Err(_) => {
                    self.remove_session(&session_id);
                    return Err(CaptureRegistryError::Capture(
                        "timed out waiting for first capture frame".to_string(),
                    ));
                }
            },
            _ = startup_cancel_rx => {
                return Err(self.startup_terminal_error(&session_id));
            }
        };

        {
            let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
            let session = inner.sessions.get_mut(&session_id).ok_or_else(|| CaptureRegistryError::Closed {
                session_id: session_id.clone(),
                message: "capture session closed during startup".to_string(),
            })?;
            match &session.lifecycle {
                CaptureSessionLifecycle::Failed(message) => {
                    return Err(CaptureRegistryError::Failed {
                        session_id: session_id.clone(),
                        message: message.clone(),
                    });
                }
                CaptureSessionLifecycle::Closed(message) => {
                    return Err(CaptureRegistryError::Closed {
                        session_id: session_id.clone(),
                        message: message.clone(),
                    });
                }
                _ => {}
            }
            session.startup_cancel = None;
        }
        startup.session_id = None;
        tracing::debug!(
            session_id = %session_id,
            width = first_frame.width,
            height = first_frame.height,
            stride = first_frame.stride,
            pixel_format = ?first_frame.pixel_format,
            "capture publisher produced first frame"
        );

        Ok(CreateCaptureSessionResponse {
            session_id,
            source_id: source_id.get(),
            track_id: track_id.get(),
            status: CaptureSessionLifecycle::Ready.status_name().to_string(),
            status_message: CaptureSessionLifecycle::Ready.status_message(),
            fd_socket_path,
            native: None,
        })
    }

    #[cfg(unix)]
    async fn create_surface_session_with_owned_frames(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
        owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        let fd_socket_path = self.fd_socket_path()?;
        let mut capture = adapter
            .start_video_capture(&surface)
            .await
            .map_err(CaptureRegistryError::from_porthole)?;
        let first_frame = tokio::time::timeout(std::time::Duration::from_secs(5), capture.next_frame())
            .await
            .map_err(|_| CaptureRegistryError::Capture("timed out waiting for first capture frame".to_string()))?
            .map_err(CaptureRegistryError::from_porthole)?
            .ok_or_else(|| CaptureRegistryError::Capture("capture stream ended before first frame".to_string()))?;

        let session_id = Uuid::new_v4().to_string();
        let mut state = SessionState::new();
        let source_id = state
            .register_source(SourceDesc {
                kind: SourceKind::Window,
                label: surface.title.clone().unwrap_or_else(|| surface.id.to_string()),
            })
            .map_err(CaptureRegistryError::from_capture)?;
        let pixel_format = capture_pixel_format(first_frame.pixel_format);
        let track_id = state
            .register_track(
                source_id,
                TrackDesc::Video(VideoTrackDesc {
                    width: first_frame.width,
                    height: first_frame.height,
                    pixel_format,
                }),
            )
            .map_err(CaptureRegistryError::from_capture)?;
        // SessionState gives us typed id allocation and validation here; the
        // current capture-session registry stores the resulting ids directly
        // until event replay is exposed over the daemon boundary.
        // TODO: retain or replay these events once daemon consumers subscribe
        // to generic session setup instead of synthesizing attach events.

        let cpu = self.new_cpu_session(&session_id)?;
        publish_capture_frame_view(&cpu, first_frame.as_view())?;

        let output_control = capture.output_control();
        let task = tokio::spawn(run_capture_session_monitor(self.clone(), session_id.clone(), capture));

        let session = CaptureSession {
            source_id,
            track_id,
            owner_agent_id: Some(owner_agent_id),
            surface_id: Some(surface.id.to_string()),
            lifecycle: CaptureSessionLifecycle::Ready,
            width: first_frame.width,
            height: first_frame.height,
            stride: first_frame.stride,
            pixel_format,
            cpu: Some(cpu),
            capture_task: Some(task),
            startup_cancel: None,
            output_control,
        };
        self.inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .insert(session_id.clone(), session);

        Ok(CreateCaptureSessionResponse {
            session_id,
            source_id: source_id.get(),
            track_id: track_id.get(),
            status: CaptureSessionLifecycle::Ready.status_name().to_string(),
            status_message: CaptureSessionLifecycle::Ready.status_message(),
            fd_socket_path,
            native: None,
        })
    }

    fn remove_session(&self, session_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            #[cfg(target_os = "macos")]
            if let Some(hold) = inner.native_holds.get_mut(session_id) {
                hold.close();
                if let Some(session) = inner.sessions.get_mut(session_id) {
                    session.lifecycle = CaptureSessionLifecycle::Closed("native capture is draining".to_owned());
                }
                return;
            }
            #[cfg(unix)]
            if let Some(session) = inner.sessions.get_mut(session_id)
                && session.cpu.is_some()
            {
                session.stop_capture();
                session.lifecycle = CaptureSessionLifecycle::Closed("CPU capture is draining".to_owned());
                return;
            }
            inner.sessions.remove(session_id);
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            inner.native_holds.remove(session_id);
        }
    }

    pub fn close_session(&self, session_id: &str) -> Result<(), CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        #[cfg(target_os = "macos")]
        if let Some(hold) = inner.native_holds.get_mut(session_id) {
            hold.close();
            if let Some(session) = inner.sessions.get_mut(session_id) {
                session.lifecycle = CaptureSessionLifecycle::Closed("native capture is draining".to_owned());
            }
            return Ok(());
        }
        #[cfg(unix)]
        if let Some(session) = inner.sessions.get_mut(session_id)
            && session.cpu.is_some()
        {
            session.stop_capture();
            session.lifecycle = CaptureSessionLifecycle::Closed("CPU capture is draining".to_owned());
            return Ok(());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        inner.native_holds.remove(session_id);
        inner
            .sessions
            .remove(session_id)
            .map(|_| ())
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))
    }

    /// Create a macOS native (IOSurface/Metal) capture session. See
    /// [`native_session`].
    #[cfg(target_os = "macos")]
    pub async fn create_native_surface_session(
        &self,
        surface: SurfaceInfo,
        owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        native_session::create(self, surface, owner_agent_id).await
    }

    #[cfg(target_os = "linux")]
    pub async fn create_native_surface_session(
        &self,
        kwin_adapter: Arc<porthole_adapter_kwin::KWinAdapter>,
        surface: SurfaceInfo,
        owner_agent_id: AgentId,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        native_session_linux::create(self, kwin_adapter, surface, owner_agent_id).await
    }

    fn mark_session_failed(&self, session_id: &str, message: String) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(session) = inner.sessions.get_mut(session_id)
        {
            if let Some(cancel) = session.startup_cancel.take() {
                let _ = cancel.send(());
            }
            #[cfg(unix)]
            if let Some(cpu) = &session.cpu {
                cpu.fail(message.clone());
            }
            session.lifecycle = CaptureSessionLifecycle::Failed(message);
        }
    }

    fn mark_session_closed(&self, session_id: &str, message: String) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(session) = inner.sessions.get_mut(session_id)
        {
            if let Some(cancel) = session.startup_cancel.take() {
                let _ = cancel.send(());
            }
            session.lifecycle = CaptureSessionLifecycle::Closed(message);
            #[cfg(unix)]
            if let Some(cpu) = &session.cpu {
                cpu.stop();
            }
        }
    }

    fn startup_terminal_error(&self, session_id: &str) -> CaptureRegistryError {
        let Ok(inner) = self.inner.lock() else {
            return CaptureRegistryError::Poisoned;
        };
        match inner.sessions.get(session_id).map(|session| &session.lifecycle) {
            Some(CaptureSessionLifecycle::Failed(message)) => CaptureRegistryError::Failed {
                session_id: session_id.to_string(),
                message: message.clone(),
            },
            Some(CaptureSessionLifecycle::Closed(message)) => CaptureRegistryError::Closed {
                session_id: session_id.to_string(),
                message: message.clone(),
            },
            Some(CaptureSessionLifecycle::Starting | CaptureSessionLifecycle::Ready) => {
                tracing::warn!(
                    session_id,
                    lifecycle = ?inner.sessions.get(session_id).map(|session| &session.lifecycle),
                    "startup cancellation fired while session was not terminal"
                );
                CaptureRegistryError::Closed {
                    session_id: session_id.to_string(),
                    message: "capture session closed during startup".to_string(),
                }
            }
            None => CaptureRegistryError::Closed {
                session_id: session_id.to_string(),
                message: "capture session closed during startup".to_string(),
            },
        }
    }

    fn authorize_fd_connection(&self, session_id: &str, bearer_token: &str) -> Result<AgentId, CaptureRegistryError> {
        let agent_store = self.agent_store.as_ref().ok_or_else(|| {
            CaptureRegistryError::from_porthole(PortholeError::new(
                ErrorCode::AgentIdentityRequired,
                "capture transfer bearer token cannot be authenticated by this registry",
            ))
        })?;
        let agent_id = agent_store
            .authenticate_agent_token_blocking(bearer_token)
            .map_err(|error| CaptureRegistryError::Io(error.to_string()))?
            .ok_or_else(|| {
                CaptureRegistryError::from_porthole(PortholeError::new(
                    ErrorCode::AgentIdentityRequired,
                    "capture transfer bearer token is required",
                ))
            })?;
        self.require_fd_session_access(session_id, Some(&agent_id))?;
        Ok(agent_id)
    }

    fn require_fd_session_access(&self, session_id: &str, authorized_agent_id: Option<&AgentId>) -> Result<(), CaptureRegistryError> {
        let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))?;
        match (&session.owner_agent_id, authorized_agent_id) {
            (None, _) => Ok(()),
            (Some(owner), Some(agent_id)) if owner == agent_id => Ok(()),
            (Some(_), _) => Err(CaptureRegistryError::from_porthole(PortholeError::new(
                ErrorCode::AgentIdentityRequired,
                "capture transfer connection is not authorized for this session",
            ))),
        }
    }

    /// Controls are reserved to the authenticated creator of the session.
    /// Never hold the registry mutex across a backend call: capture callbacks
    /// publish through this same registry while the update is in progress.
    pub async fn set_output_size(
        &self,
        session_id: &str,
        agent_id: &AgentId,
        size: VideoCaptureOutputSize,
    ) -> Result<(), CaptureRegistryError> {
        let control = {
            let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
            let session = inner
                .sessions
                .get(session_id)
                .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_owned()))?;
            if session.owner_agent_id.as_ref().is_some_and(|owner| owner != agent_id) {
                return Err(CaptureRegistryError::from_porthole(PortholeError::new(
                    ErrorCode::AgentPermissionDenied,
                    "only the capture session owner may change its output",
                )));
            }
            if session.lifecycle != CaptureSessionLifecycle::Ready {
                return Err(CaptureRegistryError::NotReady {
                    session_id: session_id.to_owned(),
                    status: session.lifecycle.status_name(),
                });
            }
            session.output_control.clone().ok_or_else(|| {
                CaptureRegistryError::from_porthole(PortholeError::new(
                    ErrorCode::AdapterUnsupported,
                    "this capture session does not support output size requests",
                ))
            })?
        };
        // Current BGRA host pools have eight resources and a 512 MiB budget.
        // Reserve 1 MiB for pool metadata and conservatively align rows to 256
        // bytes. This is a request ceiling, not a promise of immediate admission:
        // old leases still count and can delay replacement within Jackstay.
        let row = u64::from(size.width) * 4;
        let frame_bytes = (row.div_ceil(256) * 256).checked_mul(u64::from(size.height));
        if size.width == 0
            || size.height == 0
            || frame_bytes.is_none_or(|bytes| bytes > (CAPTURE_MEMORY_BUDGET - 1024 * 1024) / u64::from(CAPTURE_RESOURCE_CAPACITY))
        {
            return Err(CaptureRegistryError::from_porthole(PortholeError::new(
                ErrorCode::InvalidArgument,
                "capture output must have positive pixel dimensions and fit the session pool budget",
            )));
        }
        control.set_output_size(size).await.map_err(CaptureRegistryError::from_porthole)
    }

    /// Inserts a ready session record for route tests, with an optional owner
    /// and optional native attach info, without any backend.
    #[cfg(test)]
    pub(crate) fn insert_test_session(
        &self,
        session_id: &str,
        owner: Option<AgentId>,
        surface_id: Option<String>,
        native: Option<porthole_protocol::capture_sessions::NativeCaptureInfo>,
    ) {
        let mut inner = self.inner.lock().expect("registry poisoned");
        inner.sessions.insert(
            session_id.to_owned(),
            CaptureSession {
                source_id: SourceId::new(1),
                track_id: TrackId::new(1),
                owner_agent_id: owner,
                surface_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 8,
                height: 8,
                stride: 32,
                pixel_format: PixelFormat::Bgra8Unorm,
                #[cfg(unix)]
                cpu: None,
                capture_task: None,
                startup_cancel: None,
                output_control: None,
            },
        );
        if let Some(native) = native {
            inner.test_native_info.insert(session_id.to_owned(), native);
        }
    }

    /// Every session as a response, with the surface it captures (the session
    /// id itself when the session has no surface).
    pub fn list_sessions(&self) -> Result<Vec<(CaptureSessionResponse, String)>, CaptureRegistryError> {
        let ids: Vec<String> = self
            .inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .keys()
            .cloned()
            .collect();
        // A session can close between the snapshot and its lookup; omit it
        // rather than failing the whole listing.
        let mut sessions = Vec::with_capacity(ids.len());
        for id in &ids {
            match self.session_with_source(id) {
                Ok(entry) => sessions.push(entry),
                Err(CaptureRegistryError::UnknownSession(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(sessions)
    }

    /// A session's response plus the identity of what it captures.
    pub fn session_with_source(&self, session_id: &str) -> Result<(CaptureSessionResponse, String), CaptureRegistryError> {
        let response = self.get_session(session_id)?;
        let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let source = inner
            .sessions
            .get(session_id)
            .and_then(|s| s.surface_id.clone())
            .unwrap_or_else(|| session_id.to_owned());
        Ok((response, source))
    }

    /// The agent that created the session, if any.
    pub fn session_owner(&self, session_id: &str) -> Result<Option<AgentId>, CaptureRegistryError> {
        let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        inner
            .sessions
            .get(session_id)
            .map(|s| s.owner_agent_id.clone())
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))
    }

    pub fn get_session(&self, session_id: &str) -> Result<CaptureSessionResponse, CaptureRegistryError> {
        let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))?;
        #[cfg(target_os = "macos")]
        let native = inner.native_holds.get(session_id).map(|hold| hold.native_info.clone());
        #[cfg(not(target_os = "macos"))]
        let native: Option<porthole_protocol::capture_sessions::NativeCaptureInfo> = None;
        #[cfg(test)]
        let native = native.or_else(|| inner.test_native_info.get(session_id).cloned());
        // Native sessions are consumed over XPC, not the fd socket; tolerate
        // the socket being unconfigured for them.
        let fd_socket_path = if native.is_some() {
            self.fd_socket_path().unwrap_or_default()
        } else {
            self.fd_socket_path()?
        };
        let response = CaptureSessionResponse {
            session_id: session_id.to_string(),
            source_id: session.source_id.get(),
            track_id: session.track_id.get(),
            status: session.lifecycle.status_name().to_string(),
            status_message: session.lifecycle.status_message(),
            width: session.width,
            height: session.height,
            stride: session.stride,
            pixel_format: pixel_format_name(session.pixel_format).to_string(),
            fd_socket_path,
            native,
        };
        #[cfg(unix)]
        let response = if let Some(cpu) = &session.cpu {
            let snapshot = cpu.snapshot();
            CaptureSessionResponse {
                status: snapshot.status.to_owned(),
                status_message: Some(match session.lifecycle.status_message() {
                    Some(message) => format!("{message}; {}", snapshot.message),
                    None => snapshot.message,
                }),
                width: snapshot.width,
                height: snapshot.height,
                stride: snapshot.stride,
                pixel_format: pixel_format_name(snapshot.pixel_format).to_owned(),
                ..response
            }
        } else {
            response
        };
        #[cfg(target_os = "macos")]
        let response = if let Some(hold) = inner.native_holds.get(session_id) {
            let snapshot = hold.snapshot();
            CaptureSessionResponse {
                status: snapshot.status.to_owned(),
                status_message: snapshot.message,
                width: snapshot.width,
                height: snapshot.height,
                ..response
            }
        } else {
            response
        };
        Ok(response)
    }

    #[cfg(unix)]
    fn publish_capture_frame(&self, session_id: &str, frame: VideoCaptureFrame) -> Result<(), CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))?;
        session.width = frame.width;
        session.height = frame.height;
        session.stride = frame.stride;
        session.pixel_format = capture_pixel_format(frame.pixel_format);
        if !matches!(
            session.lifecycle,
            CaptureSessionLifecycle::Starting | CaptureSessionLifecycle::Ready
        ) {
            return Err(CaptureRegistryError::Closed {
                session_id: session_id.to_owned(),
                message: "capture is terminal".to_owned(),
            });
        }
        publish_capture_frame_view(session.cpu.as_ref().expect("CPU session"), frame.as_view()).map(|_| ())
    }

    fn fd_socket_path(&self) -> Result<String, CaptureRegistryError> {
        self.fd_socket_path
            .as_ref()
            .map(|path| path.display().to_string())
            .ok_or(CaptureRegistryError::FdSocketDisabled)
    }
}

#[cfg(unix)]
async fn run_capture_session_monitor(registry: CaptureRegistry, task_session_id: String, mut capture: Box<dyn VideoCaptureSession>) {
    loop {
        match capture.next_frame().await {
            Ok(Some(frame)) => {
                if let Err(error) = registry.publish_capture_frame(&task_session_id, frame) {
                    registry.mark_session_failed(&task_session_id, error.to_string());
                    break;
                }
            }
            Ok(None) => {
                registry.mark_session_closed(&task_session_id, "capture stream ended".to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(session_id = %task_session_id, error = %error, "capture stream stopped with error");
                registry.mark_session_failed(&task_session_id, error.to_string());
                break;
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureRegistryError {
    #[error("capture fd socket is disabled")]
    FdSocketDisabled,

    #[error("unknown capture session {0}")]
    UnknownSession(String),

    #[error("capture session {session_id} is not ready ({status})")]
    NotReady { session_id: String, status: &'static str },

    #[error("capture session {session_id} failed: {message}")]
    Failed { session_id: String, message: String },

    #[error("capture session {session_id} is closed: {message}")]
    Closed { session_id: String, message: String },

    #[error("capture registry lock poisoned")]
    Poisoned,

    #[error("{0}")]
    Capture(String),

    #[error("{0}")]
    Porthole(porthole_core::PortholeError),

    #[error("io error: {0}")]
    Io(String),
}

impl CaptureRegistryError {
    pub(crate) fn from_capture(error: jackstay::CaptureTransferError) -> Self {
        Self::Capture(error.to_string())
    }

    pub(crate) fn from_porthole(error: porthole_core::PortholeError) -> Self {
        Self::Porthole(error)
    }
}

#[cfg(unix)]
fn publish_capture_frame_view(
    cpu: &cpu_session::CpuSession,
    frame: VideoCaptureFrameView<'_>,
) -> Result<PublishOutcome, CaptureRegistryError> {
    cpu.publish(frame_descriptor_from_capture(frame.metadata), frame.bytes)
}

#[cfg(unix)]
fn frame_descriptor_from_capture(metadata: VideoCaptureFrameMetadata) -> FrameDescriptor {
    FrameDescriptor {
        sequence: metadata.sequence,
        timestamp_ns: metadata.timestamp_ns,
        width: metadata.width,
        height: metadata.height,
        stride: metadata.stride,
        pixel_format: capture_pixel_format(metadata.pixel_format) as u32,
        clock_domain: capture_clock_domain(metadata.timestamp_clock) as u32,
        color_space: capture_color_space(metadata.color_space) as u32,
        sync_kind: FrameSyncKind::CpuCopyComplete as u32,
        damage_kind: capture_damage_kind(metadata.damage_kind) as u32,
        damage_base_sequence: metadata.damage_base_sequence,
        dropped_before_publish: metadata.dropped_before_publish.try_into().unwrap_or(u32::MAX),
        producer_drop_count: metadata.producer_drop_count,
        payload_kind: PayloadKind::CpuShm as u32,
        ..FrameDescriptor::default()
    }
}

fn capture_pixel_format(format: VideoCapturePixelFormat) -> PixelFormat {
    match format {
        VideoCapturePixelFormat::Bgra8Unorm => PixelFormat::Bgra8Unorm,
    }
}

fn capture_clock_domain(domain: VideoCaptureTimestampClock) -> ClockDomain {
    match domain {
        VideoCaptureTimestampClock::Unknown => ClockDomain::Unknown,
        VideoCaptureTimestampClock::UnixTime => ClockDomain::UnixTime,
        VideoCaptureTimestampClock::MediaTime => ClockDomain::MediaTime,
        VideoCaptureTimestampClock::HostTime => ClockDomain::HostTime,
    }
}

fn capture_color_space(color_space: VideoCaptureColorSpace) -> ColorSpace {
    match color_space {
        VideoCaptureColorSpace::Unknown => ColorSpace::Unknown,
        VideoCaptureColorSpace::Srgb => ColorSpace::Srgb,
    }
}

fn capture_damage_kind(damage_kind: VideoCaptureDamageKind) -> DamageKind {
    match damage_kind {
        VideoCaptureDamageKind::Unknown => DamageKind::Unknown,
        VideoCaptureDamageKind::FullFrame => DamageKind::FullFrame,
        VideoCaptureDamageKind::None => DamageKind::None,
    }
}

#[cfg(unix)]
fn spawn_fd_listener(listener: UnixListener, registry: CaptureRegistry) {
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let registry = registry.clone();
            thread::spawn(move || {
                let _ = handle_fd_connection(stream, registry);
            });
        }
    });
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum CpuOpenRequest {
    OpenCpuAcquisition {
        session_id: String,
        track_id: u64,
        bearer_token: Option<String>,
    },
}

#[cfg(unix)]
fn handle_fd_connection(mut stream: UnixStream, registry: CaptureRegistry) -> Result<(), CaptureRegistryError> {
    stream
        .set_nonblocking(false)
        .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    // Do not buffer the following binary Jackstay setup message.
    let result = (|| {
        let mut bytes = Vec::new();
        loop {
            if bytes.len() == 16 * 1024 {
                return Err(CaptureRegistryError::Io("CPU session preface is too large".to_owned()));
            }
            let mut byte = [0];
            stream
                .read_exact(&mut byte)
                .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
            if byte == *b"\n" {
                break;
            }
            bytes.push(byte[0]);
        }
        let CpuOpenRequest::OpenCpuAcquisition {
            session_id,
            track_id,
            bearer_token,
        } = serde_json::from_slice(&bytes).map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
        let authorized = bearer_token
            .as_ref()
            .map(|token| registry.authorize_fd_connection(&session_id, token))
            .transpose()?;
        registry.require_fd_session_access(&session_id, authorized.as_ref())?;
        let inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get(&session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.clone()))?;
        match &session.lifecycle {
            CaptureSessionLifecycle::Starting => {
                return Err(CaptureRegistryError::NotReady {
                    session_id,
                    status: "starting",
                });
            }
            CaptureSessionLifecycle::Failed(message) => {
                return Err(CaptureRegistryError::Failed {
                    session_id,
                    message: message.clone(),
                });
            }
            CaptureSessionLifecycle::Closed(message) => {
                return Err(CaptureRegistryError::Closed {
                    session_id,
                    message: message.clone(),
                });
            }
            CaptureSessionLifecycle::Ready => {}
        }
        if session.track_id.get() != track_id {
            return Err(CaptureRegistryError::Capture("unknown capture track".to_owned()));
        }
        session
            .cpu
            .as_ref()
            .ok_or_else(|| CaptureRegistryError::Capture("session is not a CPU capture".to_owned()))?
            .open_connection(&stream)
    })();
    let (producer, _connection) = match result {
        Ok(ready) => ready,
        Err(error) => {
            let reply = serde_json::json!({ "op": "rejected", "message": error.to_string() });
            let _ = writeln!(stream, "{reply}");
            return Err(error);
        }
    };
    writeln!(stream, "{{\"op\":\"cpu_opened\"}}").map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    jackstay::acquisition::socket::serve_cpu(stream, producer).map_err(|error| CaptureRegistryError::Capture(error.to_string()))
}

fn pixel_format_name(format: PixelFormat) -> &'static str {
    match format {
        PixelFormat::Unknown => "unknown",
        PixelFormat::Bgra8Unorm => "bgra8_unorm",
        PixelFormat::Rgba8Unorm => "rgba8_unorm",
    }
}

#[cfg(all(test, unix))]
mod tests;
