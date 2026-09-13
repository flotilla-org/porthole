use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    time::Duration,
};

use async_trait::async_trait;
use jackstay::acquisition::{arena::AcquireOutcome, socket::CpuSetupClient};
use porthole_core::adapter::VideoCaptureSyncKind;

use super::*;

fn metadata(sequence: u64) -> VideoCaptureFrameMetadata {
    VideoCaptureFrameMetadata {
        sequence,
        timestamp_ns: sequence,
        timestamp_clock: VideoCaptureTimestampClock::MediaTime,
        width: 1,
        height: 1,
        stride: 4,
        pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
        color_space: VideoCaptureColorSpace::Srgb,
        sync_kind: VideoCaptureSyncKind::SckSampleReady,
        damage_kind: VideoCaptureDamageKind::FullFrame,
        damage_base_sequence: sequence,
        dropped_before_publish: 0,
        producer_drop_count: 0,
    }
}

fn insert_session(registry: &CaptureRegistry, id: &str, owner: Option<AgentId>, lifecycle: CaptureSessionLifecycle) {
    let cpu = cpu_session::CpuSession::new().unwrap();
    cpu.publish(frame_descriptor_from_capture(metadata(1)), b"held").unwrap();
    registry.inner.lock().unwrap().sessions.insert(
        id.to_owned(),
        CaptureSession {
            source_id: SourceId::new(1),
            track_id: TrackId::new(1),
            owner_agent_id: owner,
            lifecycle,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu),
            capture_task: None,
            startup_cancel: None,
            output_control: None,
        },
    );
}

fn open(
    registry: &CaptureRegistry,
    id: &str,
    track: u64,
    token: Option<&str>,
) -> (UnixStream, serde_json::Value, thread::JoinHandle<Result<(), CaptureRegistryError>>) {
    let (server, mut stream) = UnixStream::pair().unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    let served = registry.clone();
    let task = thread::spawn(move || handle_fd_connection(server, served));
    let request = serde_json::json!({"op":"open_cpu_acquisition", "session_id":id, "track_id":track, "bearer_token":token});
    writeln!(stream, "{request}").unwrap();
    let mut line = String::new();
    BufReader::with_capacity(1, &mut stream).read_line(&mut line).unwrap();
    (stream, serde_json::from_str(&line).unwrap(), task)
}

fn connect(stream: UnixStream) -> CpuSetupClient {
    // SAFETY: this connection selects our conforming in-process registry's
    // producer. The client never forks or forwards its process-bound mappings.
    unsafe { CpuSetupClient::from_stream(stream) }
}

#[tokio::test]
async fn abandoning_cpu_startup_aborts_capture_and_rejects_late_publication() {
    let registry = CaptureRegistry::disabled();
    insert_session(&registry, "cancelled-startup", None, CaptureSessionLifecycle::Starting);
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(std::future::pending::<()>());
    let aborted = task.abort_handle();
    {
        let mut inner = registry.inner.lock().unwrap();
        let session = inner.sessions.get_mut("cancelled-startup").unwrap();
        session.capture_task = Some(task);
        session.startup_cancel = Some(cancel_tx);
    }
    let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();
    let owner = registry.clone();
    let startup = tokio::spawn(async move {
        let _guard = CpuStartup {
            registry: owner,
            session_id: Some("cancelled-startup".into()),
        };
        armed_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    armed_rx.await.unwrap();
    startup.abort();
    assert!(startup.await.unwrap_err().is_cancelled());
    cancel_rx.await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !aborted.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let inner = registry.inner.lock().unwrap();
    let session = &inner.sessions["cancelled-startup"];
    assert!(matches!(session.lifecycle, CaptureSessionLifecycle::Closed(_)));
    assert!(session.capture_task.is_none());
    assert!(
        session
            .cpu
            .as_ref()
            .unwrap()
            .publish(frame_descriptor_from_capture(metadata(2)), b"late")
            .is_err()
    );
}

#[test]
fn failed_capture_keeps_its_reason_after_storage_retires() {
    let registry = CaptureRegistry {
        fd_socket_path: Some("/unused-paired-test-socket".into()),
        ..CaptureRegistry::disabled()
    };
    insert_session(&registry, "failure", None, CaptureSessionLifecycle::Ready);
    registry.mark_session_failed("failure", "source disappeared".into());
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let session = registry.get_session("failure").unwrap();
        assert_eq!(session.status, "failed");
        let message = session.status_message.unwrap();
        assert!(message.contains("source disappeared"));
        if message.contains("resources retired") {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cpu_setup_retains_a_successful_frame_through_history_wrap_and_session_close() {
    let registry = CaptureRegistry {
        fd_socket_path: Some("/unused-paired-test-socket".into()),
        ..CaptureRegistry::disabled()
    };
    insert_session(&registry, "session", None, CaptureSessionLifecycle::Ready);
    let (stream, reply, task) = open(&registry, "session", 1, None);
    assert_eq!(reply["op"], "cpu_opened");
    let mut connection = connect(stream);
    let consumer = connection.attach(2).unwrap();
    let AcquireOutcome::Frame(held) = consumer.acquire_latest(0).unwrap() else {
        panic!("missing held frame")
    };
    let original = *held.descriptor();
    for sequence in 2..=100 {
        registry.inner.lock().unwrap().sessions["session"]
            .cpu
            .as_ref()
            .unwrap()
            .publish(frame_descriptor_from_capture(metadata(sequence)), b"next")
            .unwrap();
    }
    assert_eq!(held.bytes(), b"held");
    assert_eq!(held.descriptor(), &original);
    registry.close_session("session").unwrap();
    assert!(matches!(consumer.acquire_latest(0).unwrap(), AcquireOutcome::Closed));
    assert_eq!(registry.get_session("session").unwrap().status, "draining");
    drop(connection);
    // Host shutdown can interrupt the setup reader or observe normal EOF.
    let _ = task.join().unwrap();
    drop(consumer);
    assert_eq!(held.bytes(), b"held");
    drop(held);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while registry.get_session("session").unwrap().status != "closed" {
        assert!(std::time::Instant::now() < deadline, "CPU session did not retire");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cpu_acquisition_preserves_ordered_gaps_exact_misses_and_holding_limits() {
    let registry = CaptureRegistry::disabled();
    insert_session(&registry, "session", None, CaptureSessionLifecycle::Ready);
    let (stream, _, task) = open(&registry, "session", 1, None);
    let mut connection = connect(stream);
    assert!(connection.attach(6).is_err());
    let consumer = connection.attach(1).unwrap();
    let AcquireOutcome::Frame(first) = consumer.acquire_next(0).unwrap() else {
        panic!("missing frame")
    };
    assert!(matches!(consumer.acquire_latest(0).unwrap(), AcquireOutcome::HoldingLimit));
    drop(first);
    for sequence in 2..=5 {
        registry.inner.lock().unwrap().sessions["session"]
            .cpu
            .as_ref()
            .unwrap()
            .publish(frame_descriptor_from_capture(metadata(sequence)), b"next")
            .unwrap();
    }
    assert!(matches!(
        consumer.acquire_next(1).unwrap(),
        AcquireOutcome::Gap { first: 2, last: 3 }
    ));
    assert!(matches!(consumer.acquire_exact(1).unwrap(), AcquireOutcome::Miss { cursor: 1 }));
    let AcquireOutcome::Frame(next) = consumer.acquire_next(3).unwrap() else {
        panic!("missing ordered frame")
    };
    assert_eq!(next.cursor(), 4);
    drop(next);
    assert!(matches!(consumer.acquire_next(5).unwrap(), AcquireOutcome::Empty));
    drop(consumer);
    drop(connection);
    task.join().unwrap().unwrap();
}

#[tokio::test]
async fn cpu_setup_enforces_session_owner_before_transferring_any_grant() {
    let store = AgentPolicyStore::open_in_memory().await.unwrap();
    let owner = store.create_identity("owner", None, 1_000).await.unwrap();
    let other = store.create_identity("other", None, 1_001).await.unwrap();
    let registry = CaptureRegistry::disabled_with_agent_policy(store);
    insert_session(&registry, "protected", Some(owner.agent_id), CaptureSessionLifecycle::Ready);
    for token in [None, Some("incorrect"), Some(other.token.as_str())] {
        let (stream, reply, task) = open(&registry, "protected", 1, token);
        assert_eq!(reply["op"], "rejected");
        assert!(reply["message"].as_str().unwrap().contains("agent_identity_required"));
        drop(stream);
        assert!(task.join().unwrap().is_err());
    }
    let (stream, reply, task) = open(&registry, "protected", 1, Some(&owner.token));
    assert_eq!(reply["op"], "cpu_opened");
    let mut connection = connect(stream);
    let consumer = connection.attach(1).unwrap();
    let AcquireOutcome::Frame(frame) = consumer.acquire_latest(0).unwrap() else {
        panic!("authorized consumer missed frame")
    };
    assert_eq!(frame.bytes(), b"held");
    drop(frame);
    drop(consumer);
    drop(connection);
    task.join().unwrap().unwrap();
}

#[test]
fn cpu_setup_checks_lifecycle_before_track_and_rejects_unknown_sessions_or_tracks() {
    let registry = CaptureRegistry::disabled();
    for (id, lifecycle, expected) in [
        ("starting", CaptureSessionLifecycle::Starting, "not ready"),
        (
            "failed",
            CaptureSessionLifecycle::Failed("capture error".to_owned()),
            "capture error",
        ),
        (
            "closed",
            CaptureSessionLifecycle::Closed("capture ended".to_owned()),
            "capture ended",
        ),
    ] {
        insert_session(&registry, id, None, lifecycle);
        let (stream, reply, task) = open(&registry, id, 999, None);
        assert_eq!(reply["op"], "rejected");
        assert!(reply["message"].as_str().unwrap().contains(expected));
        drop(stream);
        assert!(task.join().unwrap().is_err());
    }
    insert_session(&registry, "ready", None, CaptureSessionLifecycle::Ready);
    for (id, track, expected) in [("missing", 1, "unknown capture session"), ("ready", 2, "unknown capture track")] {
        let (stream, reply, task) = open(&registry, id, track, None);
        assert_eq!(reply["op"], "rejected");
        assert!(reply["message"].as_str().unwrap().contains(expected));
        drop(stream);
        assert!(task.join().unwrap().is_err());
    }
}

#[test]
fn publisher_copies_borrowed_bytes_and_preserves_capture_metadata() {
    let registry = CaptureRegistry::disabled();
    insert_session(&registry, "session", None, CaptureSessionLifecycle::Starting);
    let (first_tx, mut first_rx) = tokio::sync::oneshot::channel();
    let publisher = RegistryVideoFramePublisher::new(registry.clone(), "session".to_owned(), first_tx);
    let mut metadata = metadata(7);
    metadata.damage_base_sequence = 3;
    metadata.dropped_before_publish = 2;
    metadata.producer_drop_count = 5;
    let mut bytes = *b"copy";
    publisher.publish_frame(VideoCaptureFrameView { metadata, bytes: &bytes }).unwrap();
    bytes.fill(0);
    assert_eq!(first_rx.try_recv().unwrap().width, 1);
    assert_eq!(
        registry.inner.lock().unwrap().sessions["session"].lifecycle,
        CaptureSessionLifecycle::Ready
    );
    let (stream, _, task) = open(&registry, "session", 1, None);
    let mut connection = connect(stream);
    let consumer = connection.attach(1).unwrap();
    let AcquireOutcome::Frame(frame) = consumer.acquire_latest(0).unwrap() else {
        panic!("missing copied frame")
    };
    let desc = frame.descriptor();
    assert_eq!(frame.bytes(), b"copy");
    assert_eq!(desc.sequence, 7);
    assert_eq!(desc.clock_domain, ClockDomain::MediaTime as u32);
    assert_eq!(desc.color_space, ColorSpace::Srgb as u32);
    assert_eq!(desc.sync_kind, FrameSyncKind::CpuCopyComplete as u32);
    assert_eq!(desc.damage_kind, DamageKind::FullFrame as u32);
    assert_eq!(desc.damage_base_sequence, 3);
    assert_eq!(desc.dropped_before_publish, 2);
    assert_eq!(desc.producer_drop_count, 5);
    drop(frame);
    drop(consumer);
    drop(connection);
    task.join().unwrap().unwrap();
    registry.close_session("session").unwrap();
    assert!(publisher.publish_frame(VideoCaptureFrameView { metadata, bytes: b"late" }).is_err());
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

#[tokio::test]
async fn capture_session_monitor_marks_session_closed_when_stream_ends() {
    let registry = CaptureRegistry::disabled();
    let session_id = "owned-session".to_string();
    let source_id = jackstay::model::SourceId::new(1);
    let track_id = jackstay::model::TrackId::new(1);
    registry.inner.lock().unwrap().sessions.insert(
        session_id.clone(),
        CaptureSession {
            source_id,
            track_id,
            owner_agent_id: None,
            lifecycle: CaptureSessionLifecycle::Ready,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu_session::CpuSession::new().unwrap()),
            capture_task: None,
            startup_cancel: None,
            output_control: None,
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
    let source_id = jackstay::model::SourceId::new(1);
    let track_id = jackstay::model::TrackId::new(1);
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    registry.inner.lock().unwrap().sessions.insert(
        session_id.clone(),
        CaptureSession {
            source_id,
            track_id,
            owner_agent_id: None,
            lifecycle: CaptureSessionLifecycle::Starting,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu_session::CpuSession::new().unwrap()),
            capture_task: None,
            startup_cancel: Some(cancel_tx),
            output_control: None,
        },
    );

    registry.close_session(&session_id).unwrap();

    cancel_rx.await.unwrap();
}

#[tokio::test]
async fn capture_session_monitor_marks_session_failed_on_error() {
    let registry = CaptureRegistry::disabled();
    let session_id = "publisher-session".to_string();
    let source_id = jackstay::model::SourceId::new(1);
    let track_id = jackstay::model::TrackId::new(1);
    registry.inner.lock().unwrap().sessions.insert(
        session_id.clone(),
        CaptureSession {
            source_id,
            track_id,
            owner_agent_id: None,
            lifecycle: CaptureSessionLifecycle::Ready,
            width: 1,
            height: 1,
            stride: 4,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu_session::CpuSession::new().unwrap()),
            capture_task: None,
            startup_cancel: None,
            output_control: None,
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
    let source_id = jackstay::model::SourceId::new(1);
    let track_id = jackstay::model::TrackId::new(1);
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    registry.inner.lock().unwrap().sessions.insert(
        session_id.clone(),
        CaptureSession {
            source_id,
            track_id,
            owner_agent_id: None,
            lifecycle: CaptureSessionLifecycle::Starting,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: Some(cpu_session::CpuSession::new().unwrap()),
            capture_task: None,
            startup_cancel: Some(cancel_tx),
            output_control: None,
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

#[derive(Debug, Default)]
struct OutputControl {
    requests: Mutex<Vec<VideoCaptureOutputSize>>,
}

#[async_trait]
impl VideoCaptureOutputControl for OutputControl {
    async fn set_output_size(&self, size: VideoCaptureOutputSize) -> Result<(), PortholeError> {
        self.requests.lock().unwrap().push(size);
        Ok(())
    }
}

#[tokio::test]
async fn output_request_checks_owner_bounds_and_lifecycle_before_backend() {
    let registry = CaptureRegistry::disabled();
    let owner = AgentId::from("owner");
    insert_session(&registry, "output", Some(owner.clone()), CaptureSessionLifecycle::Ready);
    let control = Arc::new(OutputControl::default());
    registry.inner.lock().unwrap().sessions.get_mut("output").unwrap().output_control = Some(control.clone());
    let size = VideoCaptureOutputSize { width: 1920, height: 1080 };
    let error = registry.set_output_size("output", &AgentId::from("other"), size).await.unwrap_err();
    assert!(matches!(error, CaptureRegistryError::Porthole(e) if e.code == ErrorCode::AgentPermissionDenied));
    for invalid in [
        VideoCaptureOutputSize { width: 0, ..size },
        VideoCaptureOutputSize {
            width: u32::MAX,
            height: u32::MAX,
        },
    ] {
        let error = registry.set_output_size("output", &owner, invalid).await.unwrap_err();
        assert!(matches!(error, CaptureRegistryError::Porthole(e) if e.code == ErrorCode::InvalidArgument));
    }
    assert!(control.requests.lock().unwrap().is_empty());
    registry.set_output_size("output", &owner, size).await.unwrap();
    assert_eq!(*control.requests.lock().unwrap(), [size]);
    // Backend acceptance cannot manufacture published dimensions.
    assert_eq!(registry.inner.lock().unwrap().sessions["output"].width, 1);
    registry.close_session("output").unwrap();
    assert!(registry.set_output_size("output", &owner, size).await.is_err());
    assert_eq!(control.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn output_request_without_backend_control_is_explicitly_unsupported() {
    let registry = CaptureRegistry::disabled();
    let owner = AgentId::from("owner");
    insert_session(&registry, "fixed", Some(owner.clone()), CaptureSessionLifecycle::Ready);
    let error = registry
        .set_output_size("fixed", &owner, VideoCaptureOutputSize { width: 1920, height: 1080 })
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRegistryError::Porthole(e) if e.code == ErrorCode::AdapterUnsupported));
}
