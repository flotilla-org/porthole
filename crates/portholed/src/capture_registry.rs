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
    model::{PixelFormat, SourceDesc, SourceId, SourceKind, TrackDesc, TrackId, VideoTrackDesc},
    state::SessionState,
    video::{ConsumerId, VideoFrameDesc, VideoSlotManager},
};
use porthole_core::{
    adapter::{Adapter, VideoCaptureFrame, VideoCapturePixelFormat},
    surface::SurfaceInfo,
};
use porthole_protocol::capture_sessions::{
    CaptureSessionResponse, CreateSyntheticCaptureSessionResponse, LatestVideoFrameRequest, LatestVideoFrameResponse,
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

    pub fn create_synthetic_session(&self) -> Result<CreateSyntheticCaptureSessionResponse, CaptureRegistryError> {
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

        let mut video = VideoSlotManager::new(3);
        video
            .publish(
                track_id,
                VideoFrameDesc {
                    sequence: 1,
                    timestamp_ns: 0,
                    width: 2,
                    height: 1,
                    stride: 8,
                    pixel_format: PixelFormat::Bgra8Unorm,
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

        Ok(CreateSyntheticCaptureSessionResponse {
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
    ) -> Result<CreateSyntheticCaptureSessionResponse, CaptureRegistryError> {
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

        Ok(CreateSyntheticCaptureSessionResponse {
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
        let fd = frame.try_clone_fd().map_err(CaptureRegistryError::from_capture)?;
        let response = LatestVideoFrameResponse {
            session_id: request.session_id.clone(),
            track_id: request.track_id,
            sequence: frame.desc.sequence,
            timestamp_ns: frame.desc.timestamp_ns,
            width: frame.desc.width,
            height: frame.desc.height,
            stride: frame.desc.stride,
            pixel_format: pixel_format_name(frame.desc.pixel_format).to_string(),
            len: frame.bytes().len(),
        };
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
