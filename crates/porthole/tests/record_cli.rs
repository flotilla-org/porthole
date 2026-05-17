use std::{cell::RefCell, rc::Rc, time::Duration};

use porthole::{
    client::ClientError,
    commands::record::{
        FrameUnavailableReason, MovieWriter, MovieWriterSettings, OrderedFrame, OrderedFrameConsumer, RecordAcquire, RecordArgs,
        RecordFrameUnavailable, RecordSession, RecordSessionClient, RecordSummary, RecorderFactory, format_record_summary,
        parse_record_duration, record_surface_with, validate_movie_writer_settings,
    },
};

#[test]
fn parses_seconds_record_duration() {
    assert_eq!(parse_record_duration("5s").unwrap(), Duration::from_secs(5));
}

#[test]
fn parses_millisecond_record_duration() {
    assert_eq!(parse_record_duration("250ms").unwrap(), Duration::from_millis(250));
}

#[test]
fn rejects_zero_record_duration() {
    let err = parse_record_duration("0s").unwrap_err();

    assert!(err.contains("greater than zero"));
}

#[test]
fn rejects_unknown_record_duration_suffix() {
    let err = parse_record_duration("5m").unwrap_err();

    assert!(err.contains("expected duration like"));
}

#[test]
fn formats_record_summary() {
    let summary = RecordSummary {
        output: "/tmp/out.mov".into(),
        frames_written: 17,
        lapped_count: 2,
        duration: Duration::from_secs(3),
    };

    let rendered = format_record_summary(&summary);

    assert!(rendered.contains("output: /tmp/out.mov"));
    assert!(rendered.contains("frames_written: 17"));
    assert!(rendered.contains("lapped_count: 2"));
    assert!(rendered.contains("duration_ms: 3000"));
}

#[test]
fn movie_writer_validation_rejects_non_bgra_pixels() {
    let err = validate_movie_writer_settings(&MovieWriterSettings {
        output: "/tmp/out.mov".into(),
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: "rgba8_unorm".to_string(),
    })
    .unwrap_err();

    assert!(err.contains("bgra8_unorm"));
}

#[test]
fn movie_writer_validation_rejects_short_stride() {
    let err = validate_movie_writer_settings(&MovieWriterSettings {
        output: "/tmp/out.mov".into(),
        width: 10,
        height: 10,
        stride: 39,
        pixel_format: "bgra8_unorm".to_string(),
    })
    .unwrap_err();

    assert!(err.contains("stride"));
}

#[test]
fn movie_writer_validation_accepts_bgra_with_full_stride() {
    validate_movie_writer_settings(&MovieWriterSettings {
        output: "/tmp/out.mov".into(),
        width: 10,
        height: 10,
        stride: 40,
        pixel_format: "bgra8_unorm".to_string(),
    })
    .unwrap();
}

#[tokio::test]
async fn record_surface_closes_created_session_on_success() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_frames(vec![
        FakeFrame::new(1, 1_000, vec![1, 2, 3, 4]),
        FakeFrame::new(2, 1_000_001_000, vec![5, 6, 7, 8]),
    ]);

    let summary = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap();

    assert_eq!(summary.frames_written, 2);
    assert_eq!(client.closed_sessions, vec!["capture-1"]);
}

#[tokio::test]
async fn record_surface_closes_created_session_when_writer_fails() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_frames(vec![FakeFrame::new(1, 1_000, vec![1, 2, 3, 4])]);
    factory.writer_fails_after = Some(0);

    let err = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("writer failed"));
    assert_eq!(client.closed_sessions, vec!["capture-1"]);
}

#[tokio::test]
async fn record_surface_fails_on_lapped_frame_by_default() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_acquires(vec![RecordAcquire::Unavailable(RecordFrameUnavailable {
        after_producer_cursor: 4,
        oldest_available_cursor: 8,
        latest_available_cursor: 10,
        skipped_count: 3,
        reason: FrameUnavailableReason::Lapped,
    })]);

    let err = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("lapped"));
    assert_eq!(client.closed_sessions, vec!["capture-1"]);
}

#[tokio::test]
async fn record_surface_reports_close_failure_after_success() {
    let mut client = FakeSessionClient {
        next_session_id: "close-fails".to_string(),
        ..FakeSessionClient::default()
    };
    let mut factory = FakeFactory::with_frames(vec![
        FakeFrame::new(1, 1_000, vec![1, 2, 3, 4]),
        FakeFrame::new(2, 1_000_001_000, vec![5, 6, 7, 8]),
    ]);

    let err = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("close failed"));
}

#[tokio::test]
async fn record_surface_preserves_recording_error_when_close_also_fails() {
    let mut client = FakeSessionClient {
        next_session_id: "close-fails".to_string(),
        ..FakeSessionClient::default()
    };
    let mut factory = FakeFactory::with_frames(vec![FakeFrame::new(1, 1_000, vec![1, 2, 3, 4])]);
    factory.writer_fails_after = Some(0);

    let err = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("writer failed"));
}

#[tokio::test]
async fn record_surface_fails_when_no_frames_arrive_before_deadline() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_acquires(vec![]);

    let err = record_surface_with(&mut client, &mut factory, test_args(Duration::from_millis(1)))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("no video frames"));
    assert_eq!(client.closed_sessions, vec!["capture-1"]);
}

#[tokio::test]
async fn record_surface_counts_lapped_frames_when_best_effort() {
    let mut client = FakeSessionClient::default();
    let mut args = test_args(Duration::from_secs(1));
    args.best_effort = true;
    let mut factory = FakeFactory::with_acquires(vec![
        RecordAcquire::Unavailable(RecordFrameUnavailable {
            after_producer_cursor: 4,
            oldest_available_cursor: 8,
            latest_available_cursor: 10,
            skipped_count: 3,
            reason: FrameUnavailableReason::Lapped,
        }),
        RecordAcquire::Frame(FakeFrame::new(11, 1_000, vec![1, 2, 3, 4])),
        RecordAcquire::Frame(FakeFrame::new(12, 1_000_001_000, vec![5, 6, 7, 8])),
    ]);

    let summary = record_surface_with(&mut client, &mut factory, args).await.unwrap();

    assert_eq!(summary.frames_written, 2);
    assert_eq!(summary.lapped_count, 3);
}

#[tokio::test]
async fn record_surface_releases_every_written_frame() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_frames(vec![
        FakeFrame::new(1, 1_000, vec![1, 2, 3, 4]),
        FakeFrame::new(2, 1_000_001_000, vec![5, 6, 7, 8]),
    ]);

    let _summary = record_surface_with(&mut client, &mut factory, test_args(Duration::from_secs(1)))
        .await
        .unwrap();

    assert_eq!(factory.released_cursors, vec![1, 2]);
}

#[tokio::test]
async fn record_surface_extends_last_frame_to_requested_duration() {
    let mut client = FakeSessionClient::default();
    let mut factory = FakeFactory::with_frames(vec![FakeFrame::new(1, 1_000, vec![1, 2, 3, 4])]);

    let summary = record_surface_with(&mut client, &mut factory, test_args(Duration::from_millis(1)))
        .await
        .unwrap();

    assert_eq!(summary.frames_written, 2);
    assert_eq!(*factory.writer_timestamps.borrow(), vec![0, 1_000_000]);
    assert_eq!(factory.released_cursors, vec![1]);
}

fn test_args(duration: Duration) -> RecordArgs {
    RecordArgs {
        surface_id: "surface-1".to_string(),
        duration,
        output: "/tmp/out.mov".into(),
        best_effort: false,
        json: false,
    }
}

#[derive(Default)]
struct FakeSessionClient {
    next_session_id: String,
    closed_sessions: Vec<String>,
}

#[async_trait::async_trait(?Send)]
impl RecordSessionClient for FakeSessionClient {
    async fn create_surface_recording_session(&mut self, surface_id: &str) -> Result<RecordSession, ClientError> {
        assert_eq!(surface_id, "surface-1");
        Ok(RecordSession {
            session_id: if self.next_session_id.is_empty() {
                "capture-1".to_string()
            } else {
                self.next_session_id.clone()
            },
            track_id: 7,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: "bgra8_unorm".to_string(),
            fd_socket_path: "/tmp/capture-transfer.sock".to_string(),
        })
    }

    async fn close_capture_session(&mut self, session_id: &str) -> Result<(), ClientError> {
        if session_id == "close-fails" {
            return Err(ClientError::Local("close failed".to_string()));
        }
        self.closed_sessions.push(session_id.to_string());
        Ok(())
    }
}

struct FakeFactory {
    acquires: Vec<RecordAcquire<FakeFrame>>,
    released_cursors: Vec<u64>,
    writer_fails_after: Option<usize>,
    writer_timestamps: Rc<RefCell<Vec<u64>>>,
}

impl FakeFactory {
    fn with_frames(frames: Vec<FakeFrame>) -> Self {
        Self::with_acquires(frames.into_iter().map(RecordAcquire::Frame).collect())
    }

    fn with_acquires(acquires: Vec<RecordAcquire<FakeFrame>>) -> Self {
        Self {
            acquires,
            released_cursors: Vec::new(),
            writer_fails_after: None,
            writer_timestamps: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl RecorderFactory for FakeFactory {
    type Consumer<'a> = FakeConsumer<'a>;
    type Writer = FakeWriter;

    fn connect_consumer(&mut self, session: &RecordSession) -> Result<Self::Consumer<'_>, ClientError> {
        assert_eq!(session.fd_socket_path, "/tmp/capture-transfer.sock");
        Ok(FakeConsumer {
            acquires: std::mem::take(&mut self.acquires),
            released_cursors: &mut self.released_cursors,
        })
    }

    fn open_writer(&mut self, settings: &MovieWriterSettings) -> Result<Self::Writer, ClientError> {
        assert_eq!(settings.width, 1);
        assert_eq!(settings.height, 1);
        assert_eq!(settings.stride, 4);
        Ok(FakeWriter {
            appended: 0,
            fails_after: self.writer_fails_after,
            timestamps: Rc::clone(&self.writer_timestamps),
        })
    }
}

struct FakeConsumer<'a> {
    acquires: Vec<RecordAcquire<FakeFrame>>,
    released_cursors: &'a mut Vec<u64>,
}

impl OrderedFrameConsumer for FakeConsumer<'_> {
    type Frame = FakeFrame;

    fn next_frame_after(&mut self, _track_id: u64, _after_producer_cursor: u64) -> Result<RecordAcquire<Self::Frame>, ClientError> {
        if self.acquires.is_empty() {
            return Ok(RecordAcquire::Unavailable(RecordFrameUnavailable {
                after_producer_cursor: 0,
                oldest_available_cursor: 0,
                latest_available_cursor: 0,
                skipped_count: 0,
                reason: FrameUnavailableReason::Empty,
            }));
        }
        Ok(self.acquires.remove(0))
    }

    fn release_frame(&mut self, frame: Self::Frame) -> Result<(), ClientError> {
        self.released_cursors.push(frame.producer_cursor);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct FakeFrame {
    producer_cursor: u64,
    timestamp_ns: u64,
    bytes: Vec<u8>,
}

impl FakeFrame {
    fn new(producer_cursor: u64, timestamp_ns: u64, bytes: Vec<u8>) -> Self {
        Self {
            producer_cursor,
            timestamp_ns,
            bytes,
        }
    }
}

impl OrderedFrame for FakeFrame {
    fn producer_cursor(&self) -> u64 {
        self.producer_cursor
    }

    fn timestamp_ns(&self) -> u64 {
        self.timestamp_ns
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

struct FakeWriter {
    appended: usize,
    fails_after: Option<usize>,
    timestamps: Rc<RefCell<Vec<u64>>>,
}

impl MovieWriter for FakeWriter {
    fn append_frame(&mut self, timestamp_ns: u64, _bytes: &[u8]) -> Result<(), ClientError> {
        if self.fails_after == Some(self.appended) {
            return Err(ClientError::Local("writer failed".to_string()));
        }
        self.timestamps.borrow_mut().push(timestamp_ns);
        self.appended += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), ClientError> {
        Ok(())
    }
}
