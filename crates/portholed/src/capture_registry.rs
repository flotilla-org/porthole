use std::{
    collections::HashMap,
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
    video::{AcquiredVideoFrame, ConsumerId, VideoFrameDesc, VideoSlotManager},
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
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
    video: VideoSlotManager,
    _capture_task: Option<tokio::task::JoinHandle<()>>,
    _capture_handle: Option<CaptureSessionHandle>,
}

struct CaptureSessionHandle {
    _inner: Box<dyn VideoCaptureSession>,
}

impl std::fmt::Debug for CaptureSessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CaptureSessionHandle").field(&"<video-capture-session>").finish()
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
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: PixelFormat::Bgra8Unorm,
            video,
            _capture_task: None,
            _capture_handle: None,
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

        self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?.sessions.insert(
            session_id.clone(),
            CaptureSession {
                source_id,
                track_id,
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(3),
                _capture_task: None,
                _capture_handle: None,
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
        self.inner
            .lock()
            .map_err(|_| CaptureRegistryError::Poisoned)?
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(session_id.clone()))?
            ._capture_handle = Some(CaptureSessionHandle { _inner: capture });

        let first_frame = match tokio::time::timeout(std::time::Duration::from_secs(5), first_frame_rx).await {
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
        };
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

        let registry = self.clone();
        let task_session_id = session_id.clone();
        let task = tokio::spawn(async move {
            loop {
                match capture.next_frame().await {
                    Ok(Some(frame)) => {
                        let _ = registry.publish_capture_frame(&task_session_id, frame);
                    }
                    Ok(None) => break,
                    Err(error) => {
                        tracing::warn!(session_id = %task_session_id, error = %error, "capture stream stopped with error");
                        break;
                    }
                }
            }
        });

        let session = CaptureSession {
            source_id,
            track_id,
            width: first_frame.width,
            height: first_frame.height,
            stride: first_frame.stride,
            pixel_format,
            video,
            _capture_task: Some(task),
            _capture_handle: None,
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
            fd_socket_path,
        })
    }

    fn remove_session(&self, session_id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.sessions.remove(session_id);
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
            width: session.width,
            height: session.height,
            stride: session.stride,
            pixel_format: pixel_format_name(session.pixel_format).to_string(),
            fd_socket_path,
        })
    }

    fn latest_frame(&self, request: &LatestVideoFrameRequest) -> Result<LatestFrameReply, CaptureRegistryError> {
        let mut inner = self.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        inner.next_consumer_id = inner.next_consumer_id.saturating_add(1).max(1);
        let consumer_id = ConsumerId::new(inner.next_consumer_id);
        let session = inner
            .sessions
            .get_mut(&request.session_id)
            .ok_or_else(|| CaptureRegistryError::UnknownSession(request.session_id.clone()))?;
        let frame = session
            .video
            .acquire_latest(consumer_id, TrackId::new(request.track_id))
            .map_err(CaptureRegistryError::from_capture)?;
        let fd = match frame.try_clone_fd() {
            Ok(fd) => fd,
            Err(error) => {
                session.video.release(frame);
                return Err(CaptureRegistryError::from_capture(error));
            }
        };
        let response = LatestVideoFrameResponse {
            session_id: request.session_id.clone(),
            track_id: request.track_id,
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
            len: frame.bytes().len(),
        };
        Ok(LatestFrameReply { response, fd, frame })
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

struct LatestFrameReply {
    response: LatestVideoFrameResponse,
    fd: std::os::fd::OwnedFd,
    frame: AcquiredVideoFrame,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureRegistryError {
    #[error("capture fd socket is disabled")]
    FdSocketDisabled,

    #[error("unknown capture session {0}")]
    UnknownSession(String),

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
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    let request: LatestVideoFrameRequest =
        serde_json::from_str(line.trim_end()).map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    let reply = registry.latest_frame(&request)?;
    let LatestFrameReply { response, fd, frame } = reply;
    let send_result = (|| {
        fdpass::send_fd(&stream, fd.as_raw_fd()).map_err(CaptureRegistryError::from_capture)?;
        writeln!(
            stream,
            "{}",
            serde_json::to_string(&response).map_err(|error| CaptureRegistryError::Io(error.to_string()))?
        )
        .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
        stream.flush().map_err(|error| CaptureRegistryError::Io(error.to_string()))?;

        let mut release_line = String::new();
        let _ = reader.read_line(&mut release_line);
        Ok(())
    })();
    let release_result = registry.release_frame(&request.session_id, frame);
    send_result.and(release_result)
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
    use capture_transfer::{
        model::{ClockDomain, ColorSpace, DamageKind, FrameSyncKind, PixelFormat},
        video::{VideoFrameDesc, VideoSlotManager},
    };
    use porthole_core::adapter::{
        VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCaptureFrameMetadata, VideoCaptureFramePublisher,
        VideoCaptureFrameView, VideoCapturePixelFormat, VideoCaptureSyncKind, VideoCaptureTimestampClock,
    };
    use porthole_protocol::capture_sessions::LatestVideoFrameRequest;

    use crate::capture_registry::{
        CaptureRegistry, CaptureSession, RegistryVideoFramePublisher, publish_capture_frame_view_to_video, video_frame_desc_from_capture,
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
                width: 1,
                height: 1,
                stride: 4,
                pixel_format: PixelFormat::Bgra8Unorm,
                video,
                _capture_task: None,
                _capture_handle: None,
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
                width: 0,
                height: 0,
                stride: 0,
                pixel_format: PixelFormat::Bgra8Unorm,
                video: VideoSlotManager::new_reusable_pool(2),
                _capture_task: None,
                _capture_handle: None,
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
        let frame = session
            .video
            .acquire_latest(capture_transfer::video::ConsumerId::new(1), track_id)
            .unwrap();
        assert_eq!(frame.bytes(), pixels);
    }
}
