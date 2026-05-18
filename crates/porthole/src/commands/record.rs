use std::{
    fmt,
    path::PathBuf,
    time::{Duration, Instant},
};

use porthole_protocol::capture_sessions::{CaptureSessionResponse, CreateCaptureSessionResponse};

#[cfg(target_os = "macos")]
use super::record_av_writer::AvMovieWriter;
#[cfg(not(target_os = "macos"))]
use super::record_av_writer_stub::AvMovieWriter;
use crate::client::{ClientError, DaemonClient};

#[derive(Debug, Clone)]
pub struct RecordArgs {
    pub surface_id: String,
    pub duration: Duration,
    pub output: PathBuf,
    pub best_effort: bool,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSummary {
    pub output: PathBuf,
    pub frames_written: u64,
    pub lapped_count: u64,
    pub duration: Duration,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecordSession {
    pub session_id: String,
    pub track_id: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
    pub fd_socket_path: String,
    pub bearer_token: Option<String>,
}

impl fmt::Debug for RecordSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordSession")
            .field("session_id", &self.session_id)
            .field("track_id", &self.track_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .field("pixel_format", &self.pixel_format)
            .field("fd_socket_path", &self.fd_socket_path)
            .field("bearer_token", &self.bearer_token.as_deref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieWriterSettings {
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFrameUnavailable {
    pub after_producer_cursor: u64,
    pub oldest_available_cursor: u64,
    pub latest_available_cursor: u64,
    pub skipped_count: u64,
    pub reason: FrameUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameUnavailableReason {
    Empty,
    Lapped,
    Other(String),
}

impl From<String> for FrameUnavailableReason {
    fn from(reason: String) -> Self {
        match reason.as_str() {
            "empty" => Self::Empty,
            "lapped" => Self::Lapped,
            _ => Self::Other(reason),
        }
    }
}

impl std::fmt::Display for FrameUnavailableReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("empty"),
            Self::Lapped => f.write_str("lapped"),
            Self::Other(reason) => f.write_str(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordAcquire<F> {
    Frame(F),
    Unavailable(RecordFrameUnavailable),
}

#[async_trait::async_trait(?Send)]
pub trait RecordSessionClient {
    async fn create_surface_recording_session(&mut self, surface_id: &str) -> Result<RecordSession, ClientError>;
    async fn close_capture_session(&mut self, session_id: &str) -> Result<(), ClientError>;
}

pub trait OrderedFrame {
    fn producer_cursor(&self) -> u64;
    fn timestamp_ns(&self) -> u64;
    fn bytes(&self) -> &[u8];
}

pub trait OrderedFrameConsumer {
    type Frame: OrderedFrame;

    fn next_frame_after(&mut self, track_id: u64, after_producer_cursor: u64) -> Result<RecordAcquire<Self::Frame>, ClientError>;
    fn release_frame(&mut self, frame: Self::Frame) -> Result<(), ClientError>;
}

pub trait MovieWriter {
    fn append_frame(&mut self, timestamp_ns: u64, bytes: &[u8]) -> Result<(), ClientError>;
    fn finish(&mut self) -> Result<(), ClientError>;
}

pub trait RecorderFactory {
    type Consumer<'a>: OrderedFrameConsumer + 'a
    where
        Self: 'a;
    type Writer: MovieWriter;

    fn connect_consumer(&mut self, session: &RecordSession) -> Result<Self::Consumer<'_>, ClientError>;
    fn open_writer(&mut self, settings: &MovieWriterSettings) -> Result<Self::Writer, ClientError>;
}

pub fn parse_record_duration(raw: &str) -> Result<Duration, String> {
    let duration = if let Some(ms) = raw.strip_suffix("ms") {
        let value = ms.parse::<u64>().map_err(|_| "expected duration like 250ms or 5s".to_string())?;
        Duration::from_millis(value)
    } else if let Some(seconds) = raw.strip_suffix('s') {
        let value = seconds
            .parse::<u64>()
            .map_err(|_| "expected duration like 250ms or 5s".to_string())?;
        Duration::from_secs(value)
    } else {
        return Err("expected duration like 250ms or 5s".to_string());
    };

    if duration.is_zero() {
        return Err("record duration must be greater than zero".to_string());
    }

    Ok(duration)
}

pub fn validate_movie_writer_settings(settings: &MovieWriterSettings) -> Result<(), String> {
    if settings.pixel_format != "bgra8_unorm" {
        return Err(format!(
            "record movie writer requires bgra8_unorm pixels, got {}",
            settings.pixel_format
        ));
    }
    let min_stride = settings
        .width
        .checked_mul(4)
        .ok_or_else(|| "record movie writer width is too large".to_string())?;
    if settings.stride < min_stride {
        return Err(format!(
            "record movie writer stride {} is shorter than width * 4 ({min_stride})",
            settings.stride
        ));
    }
    Ok(())
}

pub async fn record_surface_with<C, F>(client: &mut C, factory: &mut F, args: RecordArgs) -> Result<RecordSummary, ClientError>
where
    C: RecordSessionClient,
    F: RecorderFactory,
{
    let session = client.create_surface_recording_session(&args.surface_id).await?;
    let session_id = session.session_id.clone();

    let result = record_created_session(factory, &session, &args).await;
    let close_result = client.close_capture_session(&session_id).await;

    match (result, close_result) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(close_error)) => Err(close_error),
        (Err(error), Err(_)) => Err(error),
    }
}

async fn record_created_session<F>(factory: &mut F, session: &RecordSession, args: &RecordArgs) -> Result<RecordSummary, ClientError>
where
    F: RecorderFactory,
{
    let settings = MovieWriterSettings {
        output: args.output.clone(),
        width: session.width,
        height: session.height,
        stride: session.stride,
        pixel_format: session.pixel_format.clone(),
    };
    let mut writer = factory.open_writer(&settings)?;
    let mut consumer = factory.connect_consumer(session)?;
    record_ordered_frames(&mut consumer, &mut writer, session.track_id, args).await
}

async fn record_ordered_frames<C, W>(
    consumer: &mut C,
    writer: &mut W,
    track_id: u64,
    args: &RecordArgs,
) -> Result<RecordSummary, ClientError>
where
    C: OrderedFrameConsumer,
    W: MovieWriter,
{
    let deadline = Instant::now() + args.duration;
    let mut after_producer_cursor = 0;
    let mut first_timestamp_ns = None;
    let requested_duration_ns = u64::try_from(args.duration.as_nanos()).unwrap_or(u64::MAX);
    let mut last_frame_bytes = Vec::new();
    let mut last_presentation_timestamp_ns = 0;
    let mut frames_written = 0;
    let mut lapped_count = 0;

    loop {
        match consumer.next_frame_after(track_id, after_producer_cursor)? {
            RecordAcquire::Frame(frame) => {
                let producer_cursor = frame.producer_cursor();
                let frame_timestamp_ns = frame.timestamp_ns();
                let first = *first_timestamp_ns.get_or_insert(frame_timestamp_ns);
                let presentation_timestamp_ns = frame_timestamp_ns.saturating_sub(first);
                let append_result = writer.append_frame(presentation_timestamp_ns, frame.bytes());
                last_frame_bytes.clear();
                last_frame_bytes.extend_from_slice(frame.bytes());
                last_presentation_timestamp_ns = presentation_timestamp_ns;
                let release_result = consumer.release_frame(frame);
                append_result?;
                release_result?;
                after_producer_cursor = producer_cursor;
                frames_written += 1;
                if Duration::from_nanos(presentation_timestamp_ns) >= args.duration {
                    break;
                }
            }
            RecordAcquire::Unavailable(unavailable) if unavailable.reason == FrameUnavailableReason::Lapped => {
                if !args.best_effort {
                    return Err(ClientError::Local(format!(
                        "recording lapped after cursor {}: oldest available {}, latest available {}, skipped {}",
                        unavailable.after_producer_cursor,
                        unavailable.oldest_available_cursor,
                        unavailable.latest_available_cursor,
                        unavailable.skipped_count
                    )));
                }
                lapped_count += unavailable.skipped_count;
                after_producer_cursor = unavailable.oldest_available_cursor.saturating_sub(1);
            }
            RecordAcquire::Unavailable(unavailable) if unavailable.reason == FrameUnavailableReason::Empty => {
                if Instant::now() >= deadline {
                    if frames_written > 0 {
                        if last_presentation_timestamp_ns < requested_duration_ns {
                            writer.append_frame(requested_duration_ns, &last_frame_bytes)?;
                            frames_written += 1;
                        }
                        break;
                    }
                    return Err(ClientError::Local(
                        "recording produced no video frames before the deadline".to_string(),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            RecordAcquire::Unavailable(unavailable) => {
                return Err(ClientError::Local(format!("video frame unavailable: {}", unavailable.reason)));
            }
        }
    }

    writer.finish()?;
    Ok(RecordSummary {
        output: args.output.clone(),
        frames_written,
        lapped_count,
        duration: args.duration,
    })
}

pub fn format_record_summary(summary: &RecordSummary) -> String {
    format!(
        "output: {}\nframes_written: {}\nlapped_count: {}\nduration_ms: {}\n",
        summary.output.display(),
        summary.frames_written,
        summary.lapped_count,
        summary.duration.as_millis()
    )
}

#[async_trait::async_trait(?Send)]
impl RecordSessionClient for DaemonClient {
    async fn create_surface_recording_session(&mut self, surface_id: &str) -> Result<RecordSession, ClientError> {
        let created: CreateCaptureSessionResponse = self
            .post_json(&format!("/capture-sessions/surfaces/{surface_id}"), &serde_json::json!({}))
            .await?;
        let session: CaptureSessionResponse = self.get_json(&format!("/capture-sessions/{}", created.session_id)).await?;
        Ok(RecordSession {
            session_id: session.session_id,
            track_id: session.track_id,
            width: session.width,
            height: session.height,
            stride: session.stride,
            pixel_format: session.pixel_format,
            fd_socket_path: session.fd_socket_path,
            bearer_token: self.bearer_token().map(ToOwned::to_owned),
        })
    }

    async fn close_capture_session(&mut self, session_id: &str) -> Result<(), ClientError> {
        self.delete_empty(&format!("/capture-sessions/{session_id}")).await
    }
}

impl OrderedFrame for capture_transfer::daemon::DaemonFrame {
    fn producer_cursor(&self) -> u64 {
        self.producer_cursor
    }

    fn timestamp_ns(&self) -> u64 {
        self.desc.timestamp_ns
    }

    fn bytes(&self) -> &[u8] {
        self.bytes()
    }
}

pub async fn run(client: &mut DaemonClient, args: RecordArgs) -> Result<(), ClientError> {
    let json = args.json;
    let mut factory = ProductionRecorderFactory;
    let summary = record_surface_with(client, &mut factory, args).await?;
    if json {
        let text = serde_json::to_string_pretty(&serde_json::json!({
            "output": summary.output,
            "frames_written": summary.frames_written,
            "lapped_count": summary.lapped_count,
            "duration_ms": summary.duration.as_millis(),
        }))?;
        println!("{text}");
    } else {
        print!("{}", format_record_summary(&summary));
    }
    Ok(())
}

struct ProductionRecorderFactory;

struct DaemonOrderedFrameConsumer {
    inner: capture_transfer::daemon::DaemonConsumer,
}

impl RecorderFactory for ProductionRecorderFactory {
    type Consumer<'a> = DaemonOrderedFrameConsumer;
    type Writer = AvMovieWriter;

    fn connect_consumer(&mut self, session: &RecordSession) -> Result<Self::Consumer<'_>, ClientError> {
        let pixel_format = match session.pixel_format.as_str() {
            "bgra8_unorm" => capture_transfer::model::PixelFormat::Bgra8Unorm,
            "rgba8_unorm" => capture_transfer::model::PixelFormat::Rgba8Unorm,
            _ => capture_transfer::model::PixelFormat::Unknown,
        };
        let info = capture_transfer::daemon::SessionInfo {
            session_id: session.session_id.clone(),
            source_id: 0,
            track_id: session.track_id,
            width: session.width,
            height: session.height,
            stride: session.stride,
            pixel_format,
            fd_socket_path: session.fd_socket_path.clone(),
            bearer_token: session.bearer_token.clone(),
        };
        capture_transfer::daemon::DaemonConsumer::connect(info)
            .map(|inner| DaemonOrderedFrameConsumer { inner })
            .map_err(|error| ClientError::Local(error.to_string()))
    }

    fn open_writer(&mut self, settings: &MovieWriterSettings) -> Result<Self::Writer, ClientError> {
        AvMovieWriter::new(settings)
    }
}

impl OrderedFrameConsumer for DaemonOrderedFrameConsumer {
    type Frame = capture_transfer::daemon::DaemonFrame;

    fn next_frame_after(&mut self, track_id: u64, after_producer_cursor: u64) -> Result<RecordAcquire<Self::Frame>, ClientError> {
        match self.inner.next_frame_after(track_id, after_producer_cursor) {
            Ok(capture_transfer::daemon::DaemonFrameAcquire::Frame(frame)) => Ok(RecordAcquire::Frame(frame)),
            Ok(capture_transfer::daemon::DaemonFrameAcquire::Unavailable(unavailable)) => {
                Ok(RecordAcquire::Unavailable(RecordFrameUnavailable {
                    after_producer_cursor: unavailable.after_producer_cursor,
                    oldest_available_cursor: unavailable.oldest_available_cursor,
                    latest_available_cursor: unavailable.latest_available_cursor,
                    skipped_count: unavailable.skipped_count,
                    reason: unavailable.reason.into(),
                }))
            }
            Err(error) => Err(ClientError::Local(error.to_string())),
        }
    }

    fn release_frame(&mut self, frame: Self::Frame) -> Result<(), ClientError> {
        self.inner
            .release_frame(frame)
            .map_err(|error| ClientError::Local(error.to_string()))
    }
}

impl MovieWriter for AvMovieWriter {
    fn append_frame(&mut self, timestamp_ns: u64, bytes: &[u8]) -> Result<(), ClientError> {
        tokio::task::block_in_place(|| self.append(timestamp_ns, bytes))
    }

    fn finish(&mut self) -> Result<(), ClientError> {
        tokio::task::block_in_place(|| self.finish())
    }
}
