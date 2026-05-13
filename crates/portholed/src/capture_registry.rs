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
    model::{ClockDomain, ColorSpace, DamageKind, FrameSyncKind, PixelFormat, SourceDesc, SourceId, SourceKind, TrackDesc, TrackId, VideoTrackDesc},
    state::SessionState,
    video::{ConsumerId, VideoFrameDesc, VideoSlotManager},
};
use porthole_core::{
    adapter::{Adapter, VideoCaptureFrame, VideoCapturePixelFormat},
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

        let mut video = VideoSlotManager::new(3);
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

        let mut video = VideoSlotManager::new(3);
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
        // The fd is a cloned kernel reference to this frame's immutable backing
        // file. After SCM_RIGHTS transfer, daemon-side pinning is no longer
        // needed for consumer readability; pruning may unlink the path, but the
        // receiver's fd remains valid until it closes it.
        session.video.release(frame);
        Ok(LatestFrameReply { response, fd })
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
    video
        .publish(
            track_id,
            VideoFrameDesc {
                sequence: frame.sequence,
                timestamp_ns: frame.timestamp_ns,
                width: frame.width,
                height: frame.height,
                stride: frame.stride,
                pixel_format: capture_pixel_format(frame.pixel_format),
                clock_domain: ClockDomain::Unknown,
                color_space: ColorSpace::Unknown,
                sync_kind: FrameSyncKind::CpuCopyComplete,
                damage_kind: DamageKind::FullFrame,
                damage_base_sequence: frame.sequence,
                dropped_before_publish: 0,
                producer_drop_count: 0,
                evicted_count: 0,
                consumer_skipped_count: 0,
            },
            &frame.bytes,
        )
        .map_err(CaptureRegistryError::from_capture)
}

fn capture_pixel_format(format: VideoCapturePixelFormat) -> PixelFormat {
    match format {
        VideoCapturePixelFormat::Bgra8Unorm => PixelFormat::Bgra8Unorm,
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
    fdpass::send_fd(&stream, reply.fd.as_raw_fd()).map_err(CaptureRegistryError::from_capture)?;
    writeln!(
        stream,
        "{}",
        serde_json::to_string(&reply.response).map_err(|error| CaptureRegistryError::Io(error.to_string()))?
    )
    .map_err(|error| CaptureRegistryError::Io(error.to_string()))?;
    Ok(())
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
