use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{BufRead, BufReader, Write},
    os::{
        fd::AsRawFd,
        unix::net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

use capture_transfer::{
    fdpass,
    model::{
        ClockDomain, ColorSpace, DamageKind, FrameSyncKind, PixelFormat, SourceDesc, SourceId, SourceKind, TrackDesc, TrackId,
        VideoTrackDesc,
    },
    state::SessionState,
    transfer_channel::{CaptureTransferMessage, CaptureTransferRequest},
    video::{AcquiredVideoFrame, ConsumerId, VideoControlPageRegistration, VideoFrameDesc, VideoSlotManager},
};
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{
        Adapter, VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCaptureFrameMetadata, VideoCaptureFramePublisher,
        VideoCaptureFrameView, VideoCapturePixelFormat, VideoCaptureSession, VideoCaptureSyncKind, VideoCaptureTimestampClock,
    },
    surface::SurfaceInfo,
};
use porthole_protocol::capture_sessions::{
    CaptureSessionResponse, CreateCaptureSessionResponse, LatestVideoFrameRequest, LatestVideoFrameResponse,
};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct CaptureRegistry {
    inner: Arc<Mutex<CaptureRegistryInner>>,
    fd_socket_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct CaptureRegistryInner {
    sessions: HashMap<String, CaptureSession>,
    next_consumer_id: u64,
}

#[derive(Debug)]
struct CaptureSession {
    source_id: SourceId,
    track_id: TrackId,
    lifecycle: CaptureSessionLifecycle,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
    video: VideoSlotManager,
    capture_task: Option<tokio::task::JoinHandle<()>>,
    startup_cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Some(task) = self.capture_task.take() {
            task.abort();
        }
        if let Some(cancel) = self.startup_cancel.take() {
            let _ = cancel.send(());
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FirstFrameInfo {
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
}

#[derive(Debug)]
struct RegistryVideoFramePublisher {
    registry: CaptureRegistry,
    session_id: String,
    first_frame_tx: Mutex<Option<tokio::sync::oneshot::Sender<FirstFrameInfo>>>,
}

impl RegistryVideoFramePublisher {
    fn new(registry: CaptureRegistry, session_id: String, first_frame_tx: tokio::sync::oneshot::Sender<FirstFrameInfo>) -> Self {
        Self {
            registry,
            session_id,
            first_frame_tx: Mutex::new(Some(first_frame_tx)),
        }
    }
}

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
        publish_capture_frame_view_to_video(&mut session.video, session.track_id, frame)
            .map_err(|error| PortholeError::new(ErrorCode::InternalError, error.to_string()))?;
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
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CaptureRegistryInner::default())),
            fd_socket_path: None,
        }
    }

    pub fn with_fd_socket(fd_socket_path: PathBuf) -> std::io::Result<Self> {
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
        };
        spawn_fd_listener(listener, registry.clone());
        Ok(registry)
    }

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

        let mut video = VideoSlotManager::new_reusable_pool(3);
        video
            .publish(
                track_id,
                VideoFrameDesc {
                    sequence: 1,
                    // Synthetic sessions use 0 as an explicit timestamp sentinel.
                    timestamp_ns: 0,
                    width: 2,
                    height: 1,
                    stride: 8,
                    pixel_format: PixelFormat::Bgra8Unorm,
                    pool_id: 0,
                    slot_id: 0,
                    slot_generation: 0,
                    payload_offset: 0,
                    payload_len: 0,
                    payload_map_len: 0,
                    clock_domain: ClockDomain::Unknown,
                    color_space: ColorSpace::Unknown,
                    sync_kind: FrameSyncKind::CpuCopyComplete,
                    damage_kind: DamageKind::FullFrame,
                    damage_base_sequence: 1,
                    dropped_before_publish: 0,
                    producer_drop_count: 0,
                    evicted_count: 0,
                    consumer_skipped_count: 0,
                },
                &[0, 64, 128, 255, 255, 64, 128, 255],
            )
            .map_err(CaptureRegistryError::from_capture)?;

        let session = CaptureSession {
            source_id,
            track_id,
            lifecycle: CaptureSessionLifecycle::Ready,
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: PixelFormat::Bgra8Unorm,
            video,
            capture_task: None,
            startup_cancel: None,
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
        })
    }

    pub async fn create_surface_session(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
    ) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
        match self.create_surface_session_with_publisher(adapter.clone(), surface.clone()).await {
            Ok(response) => return Ok(response),
            Err(CaptureRegistryError::Porthole(error)) if error.code == ErrorCode::AdapterUnsupported => {}
            Err(error) => return Err(error),
        }

        self.create_surface_session_with_owned_frames(adapter, surface).await
    }

    async fn create_surface_session_with_publisher(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
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
                lifecycle: CaptureSessionLifecycle::Starting,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(3),
                capture_task: None,
                startup_cancel: Some(startup_cancel_tx),
            },
        );

        let (first_frame_tx, first_frame_rx) = tokio::sync::oneshot::channel();
        let publisher = Arc::new(RegistryVideoFramePublisher::new(self.clone(), session_id.clone(), first_frame_tx));
        let capture = match adapter.start_video_capture_publisher(&surface, publisher).await {
            Ok(capture) => capture,
            Err(error) => {
                self.remove_session(&session_id);
                return Err(CaptureRegistryError::from_porthole(error));
            }
        };
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
            session.capture_task = Some(task);
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

        self.inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| CaptureRegistryError::Closed {
                session_id: session_id.clone(),
                message: "capture session closed during startup".to_string(),
            })?
            .startup_cancel = None;
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
        })
    }

    async fn create_surface_session_with_owned_frames(
        &self,
        adapter: Arc<dyn Adapter>,
        surface: SurfaceInfo,
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

        let mut video = VideoSlotManager::new_reusable_pool(3);
        publish_capture_frame_to_video(&mut video, track_id, &first_frame)?;

        let task = tokio::spawn(run_capture_session_monitor(self.clone(), session_id.clone(), capture));

        let session = CaptureSession {
            source_id,
            track_id,
            lifecycle: CaptureSessionLifecycle::Ready,
            width: first_frame.width,
            height: first_frame.height,
            stride: first_frame.stride,
            pixel_format,
            video,
            capture_task: Some(task),
            startup_cancel: None,
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
        })
    }

    fn remove_session(&self, session_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.sessions.remove(session_id);
        }
    }

    pub fn close_session(&self, session_id: &str) -> Result<(), CaptureRegistryError> {
        self.inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .remove(session_id)
            .map(|_| ())
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))
    }

    fn mark_session_failed(&self, session_id: &str, message: String) {
        if let Ok(mut inner) = self.inner.lock()
            && let Some(session) = inner.sessions.get_mut(session_id)
        {
            if let Some(cancel) = session.startup_cancel.take() {
                let _ = cancel.send(());
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

    pub fn get_session(&self, session_id: &str) -> Result<CaptureSessionResponse, CaptureRegistryError> {
        let fd_socket_path = self.fd_socket_path()?;
        let inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))?;
        Ok(CaptureSessionResponse {
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
        })
    }

    fn allocate_consumer_id(&self) -> Result<ConsumerId, CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        inner.next_consumer_id = inner.next_consumer_id.saturating_add(1).max(1);
        Ok(ConsumerId::new(inner.next_consumer_id))
    }

    #[cfg(test)]
    fn latest_frame(&self, request: &LatestVideoFrameRequest) -> Result<LatestFrameReply, CaptureRegistryError> {
        let consumer_id = self.allocate_consumer_id()?;
        self.latest_frame_for_consumer(request, consumer_id, true)
    }

    fn latest_frame_for_consumer(
        &self,
        request: &LatestVideoFrameRequest,
        consumer_id: ConsumerId,
        include_control_page: bool,
    ) -> Result<LatestFrameReply, CaptureRegistryError> {
        self.frame_for_consumer(request, consumer_id, include_control_page, FrameAcquireMode::Latest)
    }

    fn frame_by_cursor_for_consumer(
        &self,
        request: &LatestVideoFrameRequest,
        consumer_id: ConsumerId,
        include_control_page: bool,
        producer_cursor: u64,
    ) -> Result<LatestFrameReply, CaptureRegistryError> {
        self.frame_for_consumer(
            request,
            consumer_id,
            include_control_page,
            FrameAcquireMode::ProducerCursor(producer_cursor),
        )
    }

    fn frame_for_consumer(
        &self,
        request: &LatestVideoFrameRequest,
        consumer_id: ConsumerId,
        include_control_page: bool,
        acquire_mode: FrameAcquireMode,
    ) -> Result<LatestFrameReply, CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(request.session_id.clone()))?;
        match &session.lifecycle {
            CaptureSessionLifecycle::Ready => {}
            CaptureSessionLifecycle::Starting => {
                return Err(CaptureRegistryError::NotReady {
                    session_id: request.session_id.clone(),
                    status: session.lifecycle.status_name(),
                });
            }
            CaptureSessionLifecycle::Failed(message) => {
                return Err(CaptureRegistryError::Failed {
                    session_id: request.session_id.clone(),
                    message: message.clone(),
                });
            }
            CaptureSessionLifecycle::Closed(message) => {
                return Err(CaptureRegistryError::Closed {
                    session_id: request.session_id.clone(),
                    message: message.clone(),
                });
            }
        }
        let track_id = TrackId::new(request.track_id);
        let frame = match acquire_mode {
            FrameAcquireMode::Latest => session.video.acquire_latest(consumer_id, track_id),
            FrameAcquireMode::ProducerCursor(producer_cursor) => session.video.acquire_cursor(consumer_id, track_id, producer_cursor),
        }
        .map_err(CaptureRegistryError::from_capture)?;
        let fd = if frame.cpu_pool_registration().is_some() {
            None
        } else {
            match frame.try_clone_fd() {
                Ok(fd) => Some(fd),
                Err(error) => {
                    session.video.release(frame);
                    return Err(CaptureRegistryError::from_capture(error));
                }
            }
        };
        let control_page = if include_control_page {
            match session.video.control_page_registration(TrackId::new(request.track_id)) {
                Ok(registration) => Some(registration),
                Err(error) => {
                    session.video.release(frame);
                    return Err(CaptureRegistryError::from_capture(error));
                }
            }
        } else {
            None
        };
        let response = LatestVideoFrameResponse {
            session_id: request.session_id.clone(),
            track_id: request.track_id,
            // The capture transfer channel owns lease allocation and overwrites this before
            // sending. Zero is deliberately reserved as "not assigned".
            lease_id: 0,
            producer_cursor: frame.producer_cursor(),
            sequence: frame.desc.sequence,
            timestamp_ns: frame.desc.timestamp_ns,
            width: frame.desc.width,
            height: frame.desc.height,
            stride: frame.desc.stride,
            pixel_format: pixel_format_name(frame.desc.pixel_format).to_string(),
            pool_id: frame.desc.pool_id,
            slot_id: frame.desc.slot_id,
            slot_generation: frame.desc.slot_generation,
            payload_offset: frame.desc.payload_offset,
            payload_len: frame.desc.payload_len,
            payload_map_len: frame.desc.payload_map_len,
            clock_domain: clock_domain_name(frame.desc.clock_domain).to_string(),
            color_space: color_space_name(frame.desc.color_space).to_string(),
            sync_kind: sync_kind_name(frame.desc.sync_kind).to_string(),
            damage_kind: damage_kind_name(frame.desc.damage_kind).to_string(),
            damage_base_sequence: frame.desc.damage_base_sequence,
            dropped_before_publish: frame.desc.dropped_before_publish,
            producer_drop_count: frame.desc.producer_drop_count,
            evicted_count: frame.desc.evicted_count,
            consumer_skipped_count: frame.desc.consumer_skipped_count,
            len: frame.bytes().len() as u64,
        };
        Ok(LatestFrameReply {
            response,
            fd,
            frame,
            control_page,
        })
    }

    fn disconnect_consumer(&self, consumer_id: ConsumerId) {
        if let Ok(mut inner) = self.inner.lock() {
            for session in inner.sessions.values_mut() {
                session.video.disconnect_consumer(consumer_id);
            }
        }
    }

    fn release_frame(&self, session_id: &str, frame: AcquiredVideoFrame) -> Result<(), CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let session = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.to_string()))?;
        session.video.release(frame);
        Ok(())
    }

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
        publish_capture_frame_to_video(&mut session.video, session.track_id, &frame)
    }

    fn fd_socket_path(&self) -> Result<String, CaptureRegistryError> {
        self.fd_socket_path
            .as_ref()
            .map(|path| path.display().to_string())
            .ok_or(CaptureRegistryError::FdSocketDisabled)
    }
}

async fn run_capture_session_monitor(registry: CaptureRegistry, task_session_id: String, mut capture: Box<dyn VideoCaptureSession>) {
    loop {
        match capture.next_frame().await {
            Ok(Some(frame)) => {
                let _ = registry.publish_capture_frame(&task_session_id, frame);
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

struct LatestFrameReply {
    response: LatestVideoFrameResponse,
    fd: Option<std::os::fd::OwnedFd>,
    frame: AcquiredVideoFrame,
    control_page: Option<VideoControlPageRegistration>,
}

#[derive(Debug)]
struct FdConnectionState {
    next_lease_id: u64,
    leases: BTreeMap<u64, (String, AcquiredVideoFrame)>,
    registered_pools: BTreeSet<RegisteredPoolKey>,
    registered_control_pages: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameAcquireMode {
    Latest,
    ProducerCursor(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RegisteredPoolKey {
    track_id: u64,
    pool_id: u64,
    pool_generation: u64,
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
    fn from_capture(error: capture_transfer::CaptureTransferError) -> Self {
        Self::Capture(error.to_string())
    }

    fn from_porthole(error: porthole_core::PortholeError) -> Self {
        Self::Porthole(error)
    }
}

fn publish_capture_frame_to_video(
    video: &mut VideoSlotManager,
    track_id: TrackId,
    frame: &VideoCaptureFrame,
) -> Result<(), CaptureRegistryError> {
    publish_capture_frame_view_to_video(video, track_id, frame.as_view())
}

fn publish_capture_frame_view_to_video(
    video: &mut VideoSlotManager,
    track_id: TrackId,
    frame: VideoCaptureFrameView<'_>,
) -> Result<(), CaptureRegistryError> {
    let mut claim = video
        .claim_video_slot(track_id, video_frame_desc_from_capture_metadata(frame.metadata), frame.bytes.len())
        .map_err(CaptureRegistryError::from_capture)?;
    claim.copy_from_slice(frame.bytes);
    video.commit_video_slot(claim).map_err(CaptureRegistryError::from_capture)
}

#[cfg(test)]
fn video_frame_desc_from_capture(frame: &VideoCaptureFrame) -> VideoFrameDesc {
    video_frame_desc_from_capture_metadata(frame.metadata())
}

fn video_frame_desc_from_capture_metadata(metadata: VideoCaptureFrameMetadata) -> VideoFrameDesc {
    VideoFrameDesc {
        sequence: metadata.sequence,
        timestamp_ns: metadata.timestamp_ns,
        width: metadata.width,
        height: metadata.height,
        stride: metadata.stride,
        pixel_format: capture_pixel_format(metadata.pixel_format),
        pool_id: 0,
        slot_id: 0,
        slot_generation: 0,
        payload_offset: 0,
        payload_len: 0,
        payload_map_len: 0,
        clock_domain: capture_clock_domain(metadata.timestamp_clock),
        color_space: capture_color_space(metadata.color_space),
        sync_kind: capture_sync_kind(metadata.sync_kind),
        damage_kind: capture_damage_kind(metadata.damage_kind),
        damage_base_sequence: metadata.damage_base_sequence,
        dropped_before_publish: metadata.dropped_before_publish,
        producer_drop_count: metadata.producer_drop_count,
        evicted_count: 0,
        consumer_skipped_count: 0,
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

fn capture_sync_kind(sync_kind: VideoCaptureSyncKind) -> FrameSyncKind {
    match sync_kind {
        VideoCaptureSyncKind::Unknown => FrameSyncKind::Unknown,
        VideoCaptureSyncKind::CpuCopyComplete => FrameSyncKind::CpuCopyComplete,
        VideoCaptureSyncKind::SckSampleReady => FrameSyncKind::SckSampleReady,
        VideoCaptureSyncKind::NativeTimeline => FrameSyncKind::NativeTimeline,
    }
}

fn capture_damage_kind(damage_kind: VideoCaptureDamageKind) -> DamageKind {
    match damage_kind {
        VideoCaptureDamageKind::Unknown => DamageKind::Unknown,
        VideoCaptureDamageKind::FullFrame => DamageKind::FullFrame,
        VideoCaptureDamageKind::None => DamageKind::None,
    }
}

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

fn handle_fd_connection(mut stream: UnixStream, registry: CaptureRegistry) -> Result<(), CaptureRegistryError> {
    let reader_stream = stream.try_clone().map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let consumer_id = registry.allocate_consumer_id()?;
    let mut connection = FdConnectionState {
        next_lease_id: 1,
        leases: BTreeMap::new(),
        // TODO: evict retired pool generations once the capture transfer channel grows pool-retirement messages.
        registered_pools: BTreeSet::new(),
        registered_control_pages: BTreeSet::new(),
    };
    let result = (|| {
        loop {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
            if bytes == 0 {
                break;
            }
            let request: CaptureTransferRequest =
                serde_json::from_str(line.trim_end()).map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
            match request {
                CaptureTransferRequest::LatestVideoFrame { session_id, track_id } => {
                    let include_control_page = !connection.registered_control_pages.contains(&track_id);
                    let request = LatestVideoFrameRequest {
                        session_id: session_id.clone(),
                        track_id,
                    };
                    let reply = registry.latest_frame_for_consumer(&request, consumer_id, include_control_page)?;
                    send_frame_reply(&mut stream, &request.session_id, include_control_page, reply, &mut connection)?;
                }
                CaptureTransferRequest::AcquireVideoFrameByCursor {
                    session_id,
                    track_id,
                    producer_cursor,
                } => {
                    let include_control_page = !connection.registered_control_pages.contains(&track_id);
                    let request = LatestVideoFrameRequest {
                        session_id: session_id.clone(),
                        track_id,
                    };
                    let reply = registry.frame_by_cursor_for_consumer(&request, consumer_id, include_control_page, producer_cursor)?;
                    send_frame_reply(&mut stream, &request.session_id, include_control_page, reply, &mut connection)?;
                }
                CaptureTransferRequest::ReleaseVideoFrame { lease_id } => {
                    if let Some((session_id, frame)) = connection.leases.remove(&lease_id) {
                        registry.release_frame(&session_id, frame)?;
                    }
                }
            }
        }
        Ok(())
    })();
    for (_, (session_id, frame)) in connection.leases {
        let _ = registry.release_frame(&session_id, frame);
    }
    registry.disconnect_consumer(consumer_id);
    result
}

fn send_frame_reply(
    stream: &mut UnixStream,
    session_id: &str,
    include_control_page: bool,
    reply: LatestFrameReply,
    connection: &mut FdConnectionState,
) -> Result<(), CaptureRegistryError> {
    let LatestFrameReply {
        mut response,
        fd,
        frame,
        control_page,
    } = reply;
    let lease_id = connection.next_lease_id;
    connection.next_lease_id = connection.next_lease_id.wrapping_add(1).max(1);
    response.lease_id = lease_id;
    if let Some(control_page) = control_page.as_ref() {
        debug_assert!(include_control_page);
        let control_track_id = control_page.track_id.get();
        connection.registered_control_pages.insert(control_track_id);
        write_capture_transfer_message(
            stream,
            &CaptureTransferMessage::RegisterVideoControlPage {
                session_id: session_id.to_string(),
                track_id: control_track_id,
                map_len: control_page.map_len,
            },
        )?;
        stream.flush().map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
        fdpass::send_fd(stream, control_page.fd.as_raw_fd()).map_err(CaptureRegistryError::from_capture)?;
    }
    if let Some(pool) = frame.cpu_pool_registration() {
        let pool_key = RegisteredPoolKey {
            track_id: pool.track_id.get(),
            pool_id: pool.pool_id,
            pool_generation: pool.pool_generation,
        };
        if connection.registered_pools.insert(pool_key) {
            let pool_fd = frame.try_clone_fd().map_err(CaptureRegistryError::from_capture)?;
            write_capture_transfer_message(
                stream,
                &CaptureTransferMessage::RegisterCpuPool {
                    session_id: session_id.to_string(),
                    track_id: pool.track_id.get(),
                    pool_id: pool.pool_id,
                    pool_generation: pool.pool_generation,
                    payload_map_len: pool.payload_map_len,
                    slot_stride: pool.slot_stride,
                    slot_count: pool.slot_count,
                },
            )?;
            stream.flush().map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
            fdpass::send_fd(stream, pool_fd.as_raw_fd()).map_err(CaptureRegistryError::from_capture)?;
        }
    }
    connection.leases.insert(lease_id, (session_id.to_string(), frame));
    write_capture_transfer_message(stream, &video_frame_message_from_response(&response))?;
    stream.flush().map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    if let Some(fd) = fd {
        fdpass::send_fd(stream, fd.as_raw_fd()).map_err(CaptureRegistryError::from_capture)?;
    }
    Ok(())
}

fn write_capture_transfer_message(stream: &mut UnixStream, message: &CaptureTransferMessage) -> Result<(), CaptureRegistryError> {
    let line = serde_json::to_string(message).map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    writeln!(stream, "{line}").map_err(|error| CaptureRegistryError::Io(error.to_string()))
}

fn video_frame_message_from_response(response: &LatestVideoFrameResponse) -> CaptureTransferMessage {
    CaptureTransferMessage::VideoFrame {
        session_id: response.session_id.clone(),
        track_id: response.track_id,
        lease_id: response.lease_id,
        producer_cursor: response.producer_cursor,
        sequence: response.sequence,
        timestamp_ns: response.timestamp_ns,
        width: response.width,
        height: response.height,
        stride: response.stride,
        pixel_format: response.pixel_format.clone(),
        pool_id: response.pool_id,
        slot_id: response.slot_id,
        slot_generation: response.slot_generation,
        payload_offset: response.payload_offset,
        payload_len: response.payload_len,
        payload_map_len: response.payload_map_len,
        clock_domain: response.clock_domain.clone(),
        color_space: response.color_space.clone(),
        sync_kind: response.sync_kind.clone(),
        damage_kind: response.damage_kind.clone(),
        damage_base_sequence: response.damage_base_sequence,
        dropped_before_publish: response.dropped_before_publish,
        producer_drop_count: response.producer_drop_count,
        evicted_count: response.evicted_count,
        consumer_skipped_count: response.consumer_skipped_count,
        len: response.len,
    }
}

fn pixel_format_name(format: PixelFormat) -> &'static str {
    match format {
        PixelFormat::Bgra8Unorm => "bgra8_unorm",
        PixelFormat::Rgba8Unorm => "rgba8_unorm",
    }
}

fn clock_domain_name(domain: ClockDomain) -> &'static str {
    match domain {
        ClockDomain::Unknown => "unknown",
        ClockDomain::UnixTime => "unix_time",
        ClockDomain::MediaTime => "media_time",
        ClockDomain::HostTime => "host_time",
    }
}

fn color_space_name(color_space: ColorSpace) -> &'static str {
    match color_space {
        ColorSpace::Unknown => "unknown",
        ColorSpace::Srgb => "srgb",
    }
}

fn sync_kind_name(sync_kind: FrameSyncKind) -> &'static str {
    match sync_kind {
        FrameSyncKind::Unknown => "unknown",
        FrameSyncKind::CpuCopyComplete => "cpu_copy_complete",
        FrameSyncKind::SckSampleReady => "sck_sample_ready",
        FrameSyncKind::NativeTimeline => "native_timeline",
    }
}

fn damage_kind_name(damage_kind: DamageKind) -> &'static str {
    match damage_kind {
        DamageKind::Unknown => "unknown",
        DamageKind::FullFrame => "full_frame",
        DamageKind::None => "none",
        DamageKind::InlineRects => "inline_rects",
        DamageKind::SidecarRects => "sidecar_rects",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        fs::File,
        io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
        os::unix::net::UnixStream,
        thread,
    };

    use async_trait::async_trait;
    use capture_transfer::{
        control_page::VideoTrackControlPage,
        model::{ClockDomain, ColorSpace, DamageKind, FrameSyncKind, PixelFormat},
        video::{ConsumerId, VideoFrameDesc, VideoSlotManager},
    };
    use porthole_core::{
        ErrorCode, PortholeError,
        adapter::{
            VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCaptureFrameMetadata, VideoCaptureFramePublisher,
            VideoCaptureFrameView, VideoCapturePixelFormat, VideoCaptureSession, VideoCaptureSyncKind, VideoCaptureTimestampClock,
        },
    };
    use porthole_protocol::capture_sessions::{LatestVideoFrameRequest, LatestVideoFrameResponse};

    use crate::capture_registry::{
        CaptureRegistry, CaptureRegistryError, CaptureSession, CaptureSessionLifecycle, RegistryVideoFramePublisher, handle_fd_connection,
        publish_capture_frame_view_to_video, run_capture_session_monitor, video_frame_desc_from_capture,
    };

    fn test_desc(sequence: u64) -> VideoFrameDesc {
        VideoFrameDesc {
            sequence,
            timestamp_ns: sequence,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: PixelFormat::Bgra8Unorm,
            pool_id: 0,
            slot_id: 0,
            slot_generation: 0,
            payload_offset: 0,
            payload_len: 0,
            payload_map_len: 0,
            clock_domain: ClockDomain::UnixTime,
            color_space: ColorSpace::Unknown,
            sync_kind: FrameSyncKind::CpuCopyComplete,
            damage_kind: DamageKind::FullFrame,
            damage_base_sequence: sequence,
            dropped_before_publish: 0,
            producer_drop_count: 0,
            evicted_count: 0,
            consumer_skipped_count: 0,
        }
    }

    fn pinned_frame_count(registry: &CaptureRegistry, session_id: &str) -> usize {
        registry
            .inner
            .lock()
            .unwrap()
            .sessions
            .get(session_id)
            .unwrap()
            .video
            .pinned_frame_count()
    }

    struct ScriptedVideoCaptureSession {
        results: VecDeque<Result<Option<VideoCaptureFrame>, PortholeError>>,
    }

    impl ScriptedVideoCaptureSession {
        fn new(results: Vec<Result<Option<VideoCaptureFrame>, PortholeError>>) -> Self {
            Self {
                results: VecDeque::from(results),
            }
        }
    }

    #[async_trait]
    impl VideoCaptureSession for ScriptedVideoCaptureSession {
        async fn next_frame(&mut self) -> Result<Option<VideoCaptureFrame>, PortholeError> {
            self.results.pop_front().unwrap_or(Ok(None))
        }
    }

    #[test]
    fn latest_frame_reply_keeps_frame_pinned_until_release() {
        let registry = CaptureRegistry::disabled();
        let session_id = "session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let mut video = VideoSlotManager::new_reusable_pool(1);
        video.publish(track_id, test_desc(1), &[1, 2, 3, 4]).unwrap();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video,
                capture_task: None,
                startup_cancel: None,
            },
        );

        let reply = registry
            .latest_frame(&LatestVideoFrameRequest {
                session_id: session_id.clone(),
                track_id: track_id.get(),
            })
            .unwrap();

        let pinned = registry
            .inner
            .lock()
            .unwrap()
            .sessions
            .get(&session_id)
            .unwrap()
            .video
            .pinned_frame_count();
        assert_eq!(pinned, 1);

        registry.release_frame(&session_id, reply.frame).unwrap();

        let pinned = registry
            .inner
            .lock()
            .unwrap()
            .sessions
            .get(&session_id)
            .unwrap()
            .video
            .pinned_frame_count();
        assert_eq!(pinned, 0);
    }

    #[test]
    fn latest_frame_for_consumer_preserves_skip_accounting() {
        let registry = CaptureRegistry::disabled();
        let session_id = "session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let mut video = VideoSlotManager::new_reusable_pool(3);
        video.publish(track_id, test_desc(1), &[1]).unwrap();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video,
                capture_task: None,
                startup_cancel: None,
            },
        );
        let consumer = ConsumerId::new(77);

        let first = registry
            .latest_frame_for_consumer(
                &LatestVideoFrameRequest {
                    session_id: session_id.clone(),
                    track_id: track_id.get(),
                },
                consumer,
                true,
            )
            .unwrap();
        assert_eq!(first.response.consumer_skipped_count, 0);
        assert!(first.control_page.is_some());
        registry.release_frame(&session_id, first.frame).unwrap();

        {
            let mut inner = registry.inner.lock().unwrap();
            let session = inner.sessions.get_mut(&session_id).unwrap();
            session.video.publish(track_id, test_desc(2), &[2]).unwrap();
            session.video.publish(track_id, test_desc(3), &[3]).unwrap();
        }

        let second = registry
            .latest_frame_for_consumer(
                &LatestVideoFrameRequest {
                    session_id: session_id.clone(),
                    track_id: track_id.get(),
                },
                consumer,
                false,
            )
            .unwrap();
        assert_eq!(second.response.sequence, 3);
        assert_eq!(second.response.consumer_skipped_count, 1);
        assert!(second.control_page.is_none());
    }

    #[test]
    fn fd_connection_disconnect_releases_outstanding_leases() {
        let registry = CaptureRegistry::disabled();
        let session_id = "session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let mut video = VideoSlotManager::new_reusable_pool(1);
        video.publish(track_id, test_desc(1), &[1, 2, 3, 4]).unwrap();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video,
                capture_task: None,
                startup_cancel: None,
            },
        );

        let (mut client, server) = UnixStream::pair().unwrap();
        let registry_for_server = registry.clone();
        let server_thread = thread::spawn(move || handle_fd_connection(server, registry_for_server).unwrap());

        writeln!(
            client,
            "{}",
            serde_json::json!({
                "op": "latest_video_frame",
                "session_id": session_id,
                "track_id": track_id.get()
            })
        )
        .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let control: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(control["op"], "register_video_control_page");
        let control_fd = capture_transfer::fdpass::recv_fd(&client).unwrap();
        let control_page = VideoTrackControlPage::map_read_only(control_fd, control["map_len"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(control_page.shadow_read_entry_for_cursor(1).unwrap().sequence, 1);

        line.clear();
        reader.read_line(&mut line).unwrap();
        let pool: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(pool["op"], "register_cpu_pool");
        let fd = capture_transfer::fdpass::recv_fd(&client).unwrap();

        line.clear();
        reader.read_line(&mut line).unwrap();
        let response: LatestVideoFrameResponse = serde_json::from_str(line.trim_end()).unwrap();
        assert_ne!(response.lease_id, 0);
        let mut file = File::from(fd);
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(response.payload_offset)).unwrap();
        file.take(response.payload_len).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, [1, 2, 3, 4]);

        assert_eq!(pinned_frame_count(&registry, "session"), 1);
        drop(reader);
        drop(client);
        server_thread.join().unwrap();

        assert_eq!(pinned_frame_count(&registry, "session"), 0);
    }

    #[test]
    fn fd_connection_acquires_requested_producer_cursor() {
        let registry = CaptureRegistry::disabled();
        let session_id = "session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let mut video = VideoSlotManager::new_reusable_pool(2);
        video.publish(track_id, test_desc(1), &[1, 2, 3, 4]).unwrap();
        video.publish(track_id, test_desc(2), &[5, 6, 7, 8]).unwrap();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video,
                capture_task: None,
                startup_cancel: None,
            },
        );

        let (mut client, server) = UnixStream::pair().unwrap();
        let registry_for_server = registry.clone();
        let server_thread = thread::spawn(move || handle_fd_connection(server, registry_for_server).unwrap());

        writeln!(
            client,
            "{}",
            serde_json::json!({
                "op": "acquire_video_frame_by_cursor",
                "session_id": session_id,
                "track_id": track_id.get(),
                "producer_cursor": 1
            })
        )
        .unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let control: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(control["op"], "register_video_control_page");
        let control_fd = capture_transfer::fdpass::recv_fd(&client).unwrap();
        let control_page = VideoTrackControlPage::map_read_only(control_fd, control["map_len"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(control_page.shadow_read_entry_for_cursor(1).unwrap().sequence, 1);

        line.clear();
        reader.read_line(&mut line).unwrap();
        let pool: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(pool["op"], "register_cpu_pool");
        let fd = capture_transfer::fdpass::recv_fd(&client).unwrap();

        line.clear();
        reader.read_line(&mut line).unwrap();
        let response: LatestVideoFrameResponse = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(response.producer_cursor, 1);
        assert_eq!(response.sequence, 1);
        assert_ne!(response.lease_id, 0);
        let mut file = File::from(fd);
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(response.payload_offset)).unwrap();
        file.take(response.payload_len).read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, [1, 2, 3, 4]);

        writeln!(
            client,
            "{}",
            serde_json::json!({
                "op": "release_video_frame",
                "lease_id": response.lease_id
            })
        )
        .unwrap();
        drop(reader);
        drop(client);
        server_thread.join().unwrap();

        assert_eq!(pinned_frame_count(&registry, "session"), 0);
    }

    #[test]
    fn latest_frame_rejects_starting_session_before_track_lookup() {
        let registry = CaptureRegistry::disabled();
        let session_id = "starting-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Starting,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: None,
            },
        );

        let result = registry.latest_frame(&LatestVideoFrameRequest {
            session_id: session_id.clone(),
            track_id: track_id.get(),
        });

        assert!(matches!(
            result,
            Err(CaptureRegistryError::NotReady {
                session_id: id,
                status: "starting",
            }) if id == session_id
        ));
    }

    #[test]
    fn latest_frame_rejects_failed_session_before_track_lookup() {
        let registry = CaptureRegistry::disabled();
        let session_id = "failed-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Failed("producer stopped".to_string()),
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: None,
            },
        );

        let result = registry.latest_frame(&LatestVideoFrameRequest {
            session_id: session_id.clone(),
            track_id: track_id.get(),
        });

        assert!(matches!(
            result,
            Err(CaptureRegistryError::Failed {
                session_id: id,
                message,
            }) if id == session_id && message == "producer stopped"
        ));
    }

    #[test]
    fn latest_frame_rejects_closed_session_before_track_lookup() {
        let registry = CaptureRegistry::disabled();
        let session_id = "closed-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Closed("capture stream ended".to_string()),
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: None,
            },
        );

        let result = registry.latest_frame(&LatestVideoFrameRequest {
            session_id: session_id.clone(),
            track_id: track_id.get(),
        });

        assert!(matches!(
            result,
            Err(CaptureRegistryError::Closed {
                session_id: id,
                message,
            }) if id == session_id && message == "capture stream ended"
        ));
    }

    #[tokio::test]
    async fn capture_session_monitor_marks_session_closed_when_stream_ends() {
        let registry = CaptureRegistry::disabled();
        let session_id = "owned-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: None,
            },
        );

        run_capture_session_monitor(
            registry.clone(),
            session_id.clone(),
            Box::new(ScriptedVideoCaptureSession::new(vec![Ok(None)])),
        )
        .await;

        let inner = registry.inner.lock().unwrap();
        let session = inner.sessions.get(&session_id).unwrap();
        assert_eq!(
            session.lifecycle,
            CaptureSessionLifecycle::Closed("capture stream ended".to_string())
        );
    }

    #[tokio::test]
    async fn close_session_sends_startup_cancel_signal() {
        let registry = CaptureRegistry::disabled();
        let session_id = "starting-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Starting,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: Some(cancel_tx),
            },
        );

        registry.close_session(&session_id).unwrap();

        cancel_rx.await.unwrap();
    }

    #[tokio::test]
    async fn capture_session_monitor_marks_session_failed_on_error() {
        let registry = CaptureRegistry::disabled();
        let session_id = "publisher-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Ready,
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: None,
            },
        );

        run_capture_session_monitor(
            registry.clone(),
            session_id.clone(),
            Box::new(ScriptedVideoCaptureSession::new(vec![Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                "source disappeared",
            ))])),
        )
        .await;

        let inner = registry.inner.lock().unwrap();
        let session = inner.sessions.get(&session_id).unwrap();
        assert_eq!(
            session.lifecycle,
            CaptureSessionLifecycle::Failed("capability_missing: source disappeared".to_string())
        );
    }

    #[tokio::test]
    async fn capture_session_monitor_wakes_startup_waiter_on_error() {
        let registry = CaptureRegistry::disabled();
        let session_id = "publisher-starting-session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Starting,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(1),
                capture_task: None,
                startup_cancel: Some(cancel_tx),
            },
        );

        run_capture_session_monitor(
            registry.clone(),
            session_id.clone(),
            Box::new(ScriptedVideoCaptureSession::new(vec![Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                "source disappeared",
            ))])),
        )
        .await;

        cancel_rx.await.unwrap();
        assert!(matches!(
            registry.startup_terminal_error(&session_id),
            CaptureRegistryError::Failed {
                session_id: id,
                message,
            } if id == session_id && message == "capability_missing: source disappeared"
        ));
    }

    #[test]
    fn video_frame_desc_from_capture_preserves_capture_metadata() {
        let frame = VideoCaptureFrame {
            sequence: 7,
            timestamp_ns: 123,
            timestamp_clock: VideoCaptureTimestampClock::MediaTime,
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: VideoCaptureColorSpace::Srgb,
            sync_kind: VideoCaptureSyncKind::SckSampleReady,
            damage_kind: VideoCaptureDamageKind::FullFrame,
            damage_base_sequence: 3,
            dropped_before_publish: 2,
            producer_drop_count: 5,
            bytes: vec![0; 8],
        };

        let desc = video_frame_desc_from_capture(&frame);

        assert_eq!(desc.sequence, 7);
        assert_eq!(desc.pixel_format, PixelFormat::Bgra8Unorm);
        assert_eq!(desc.clock_domain, ClockDomain::MediaTime);
        assert_eq!(desc.color_space, ColorSpace::Srgb);
        assert_eq!(desc.sync_kind, FrameSyncKind::SckSampleReady);
        assert_eq!(desc.damage_kind, DamageKind::FullFrame);
        assert_eq!(desc.damage_base_sequence, 3);
        assert_eq!(desc.dropped_before_publish, 2);
        assert_eq!(desc.producer_drop_count, 5);
    }

    #[test]
    fn borrowed_capture_frame_view_publishes_into_video_slots() {
        let mut video = VideoSlotManager::new_reusable_pool(2);
        let track = capture_transfer::model::TrackId::new(1);
        let pixels = [9, 8, 7, 6, 5, 4, 3, 2];
        let metadata = VideoCaptureFrameMetadata {
            sequence: 11,
            timestamp_ns: 456,
            timestamp_clock: VideoCaptureTimestampClock::MediaTime,
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: VideoCaptureColorSpace::Srgb,
            sync_kind: VideoCaptureSyncKind::SckSampleReady,
            damage_kind: VideoCaptureDamageKind::FullFrame,
            damage_base_sequence: 10,
            dropped_before_publish: 3,
            producer_drop_count: 4,
        };

        publish_capture_frame_view_to_video(&mut video, track, VideoCaptureFrameView { metadata, bytes: &pixels }).unwrap();

        let frame = video.acquire_latest(capture_transfer::video::ConsumerId::new(1), track).unwrap();
        assert_eq!(frame.desc.sequence, 11);
        assert_eq!(frame.desc.damage_base_sequence, 10);
        assert_eq!(frame.bytes(), pixels);
    }

    #[test]
    fn registry_frame_publisher_updates_session_and_signals_first_frame() {
        let registry = CaptureRegistry::disabled();
        let session_id = "session".to_string();
        let source_id = capture_transfer::model::SourceId::new(1);
        let track_id = capture_transfer::model::TrackId::new(1);
        registry.inner.lock().unwrap().sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                lifecycle: CaptureSessionLifecycle::Starting,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(2),
                capture_task: None,
                startup_cancel: None,
            },
        );
        let (first_tx, mut first_rx) = tokio::sync::oneshot::channel();
        let publisher = RegistryVideoFramePublisher::new(registry.clone(), session_id.clone(), first_tx);
        let pixels = [3, 2, 1, 0];
        let metadata = VideoCaptureFrameMetadata {
            sequence: 1,
            timestamp_ns: 10,
            timestamp_clock: VideoCaptureTimestampClock::MediaTime,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: VideoCaptureColorSpace::Unknown,
            sync_kind: VideoCaptureSyncKind::SckSampleReady,
            damage_kind: VideoCaptureDamageKind::FullFrame,
            damage_base_sequence: 1,
            dropped_before_publish: 0,
            producer_drop_count: 0,
        };

        publisher.publish_frame(VideoCaptureFrameView { metadata, bytes: &pixels }).unwrap();

        let first = first_rx.try_recv().unwrap();
        assert_eq!(first.width, 1);
        assert_eq!(first.height, 1);
        let mut inner = registry.inner.lock().unwrap();
        let session = inner.sessions.get_mut(&session_id).unwrap();
        assert_eq!(session.width, 1);
        assert_eq!(session.height, 1);
        assert_eq!(session.stride, 4);
        assert_eq!(session.lifecycle, CaptureSessionLifecycle::Ready);
        let frame = session
            .video
            .acquire_latest(capture_transfer::video::ConsumerId::new(1), track_id)
            .unwrap();
        assert_eq!(frame.bytes(), pixels);
    }
}
