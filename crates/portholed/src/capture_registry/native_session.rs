//! macOS native capture through Jackstay's common acquisition arena.
//! Porthole owns source authority, the session budget and the teardown owner.
//! One named service remains reserved while its old session drains.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use jackstay::{
    acquisition::arena::{ArenaConfig, PublishOutcome, ReconfigurationStatus},
    model::{PixelFormat, SourceDesc, SourceKind, TrackDesc, VideoTrackDesc},
    native::{
        NativeStreamParams,
        arena::NativeArenaProducer,
        macos::{MacosCapturedFrame, MacosFrameBackend, xpc::arena::XpcArenaServer},
    },
    state::SessionState,
};
use porthole_adapter_macos::{
    MacOsAdapter,
    sck_native::{NativeCapturedFrame, NativeSckCaptureStream, NativeVideoFramePublisher, start_native_window_capture},
};
use porthole_core::{ErrorCode, PortholeError, agent_policy::AgentId, surface::SurfaceInfo};
use porthole_protocol::capture_sessions::{
    CreateCaptureSessionResponse, MACOS_NATIVE_ATTACH_MACH_SERVICE, NATIVE_ATTACH_TRANSPORT_MACOS_XPC, NativeCaptureInfo,
};
use uuid::Uuid;

use super::{CaptureRegistry, CaptureRegistryError, CaptureSession, CaptureSessionLifecycle};

type SharedProducer = Arc<Mutex<NativeArenaProducer<MacosFrameBackend>>>;
const MEMORY_BUDGET: u64 = 512 * 1024 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn arena_config() -> ArenaConfig {
    ArenaConfig {
        resource_capacity: 8,
        retained_history: 2,
        producer_reserve: 1,
        payload_capacity: 0,
        memory_budget: MEMORY_BUDGET,
        max_incarnations: 4,
        drain_timeout: DRAIN_TIMEOUT,
    }
}

pub(super) struct NativeSessionHold {
    pub native_info: NativeCaptureInfo,
    server: Option<XpcArenaServer>,
    stream: Option<NativeSckCaptureStream>,
    publisher: Arc<NativeRegistryPublisher>,
}

impl std::fmt::Debug for NativeSessionHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSessionHold")
            .field("native_info", &self.native_info)
            .finish_non_exhaustive()
    }
}

impl NativeSessionHold {
    fn new(native_info: NativeCaptureInfo, publisher: Arc<NativeRegistryPublisher>) -> Result<Self, CaptureRegistryError> {
        publisher
            .maintenance()
            .map_err(|error| CaptureRegistryError::Capture(format!("start native retirement worker: {error}")))?;
        Ok(Self {
            native_info,
            server: None,
            stream: None,
            publisher,
        })
    }

    pub(super) fn close(&mut self) {
        {
            let mut state = self.publisher.state.lock().expect("native runtime poisoned");
            if state.closing.is_none() {
                state.closing = Some(Instant::now());
                if let Some(producer) = &state.producer {
                    producer.lock().expect("native producer poisoned").stop();
                }
            }
        }
        // No runtime/producer mutex may be held while SCK joins its callback
        // queue or XPC invalidation invokes a producer cleanup callback.
        drop(self.server.take());
        drop(self.stream.take());
    }

    pub(super) fn drained(&self) -> bool {
        self.publisher.state.lock().expect("native runtime poisoned").drained
    }

    pub(super) fn snapshot(&self) -> NativeSnapshot {
        self.publisher.state.lock().expect("native runtime poisoned").snapshot()
    }
}

impl Drop for NativeSessionHold {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) struct NativeSnapshot {
    pub status: &'static str,
    pub message: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
struct NativeRuntime {
    producer: Option<SharedProducer>,
    params: Option<NativeStreamParams>,
    pending: Option<NativeStreamParams>,
    pause: Option<(u64, u64)>,
    error: Option<String>,
    cleanup_error: Option<String>,
    closing: Option<Instant>,
    drained: bool,
    published: u64,
    dropped: u64,
}

impl NativeRuntime {
    fn snapshot(&self) -> NativeSnapshot {
        let (status, detail) = if self.drained {
            ("closed", Some("native resources retired".to_owned()))
        } else if let Some(error) = &self.cleanup_error {
            ("recovery_required", Some(error.clone()))
        } else if self.closing.is_some() {
            ("draining", Some("waiting for native mappings and GPU use to retire".to_owned()))
        } else if let Some(error) = &self.error {
            ("failed", Some(error.clone()))
        } else if let Some((requested, available)) = self.pause {
            (
                "paused",
                Some(format!("native replacement needs {requested} bytes; {available} available")),
            )
        } else if self.params.is_some() {
            ("ready", None)
        } else {
            ("starting", None)
        };
        let counters = format!(
            "native budget={MEMORY_BUDGET} bytes, published={}, dropped={}",
            self.published, self.dropped
        );
        NativeSnapshot {
            status,
            message: Some(detail.map_or_else(|| counters.clone(), |detail| format!("{detail}; {counters}"))),
            width: self.params.as_ref().map_or(0, |params| params.width),
            height: self.params.as_ref().map_or(0, |params| params.height),
        }
    }

    fn maintain(&mut self) {
        let Some(producer) = self.producer.as_ref().map(Arc::clone) else {
            self.drained = self.closing.is_some();
            return;
        };
        let mut guard = producer.lock().expect("native producer poisoned");
        if let Err(error) = guard.poll_cleanup() {
            self.cleanup_error = Some(error.to_string());
        } else {
            let failures = guard.cleanup_failures();
            self.cleanup_error = (!failures.is_empty()).then(|| {
                failures
                    .into_iter()
                    .map(|failure| format!("{:?}: {}", failure.incarnation, failure.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            });
        }
        if self.closing.is_none() && self.error.is_none() && self.pending.is_some() {
            match guard.advance_reconfiguration() {
                Ok(ReconfigurationStatus::Ready { .. }) => {
                    self.params = self.pending.take();
                    self.pause = None;
                }
                Ok(ReconfigurationStatus::PausedCapacity { requested, available }) => {
                    self.pause = Some((requested, available));
                }
                Err(error) => {
                    self.error = Some(error.to_string());
                    guard.stop();
                }
            }
        }
        if let Some(started) = self.closing {
            let ready = match guard.poll_shutdown_ready() {
                Ok(ready) => ready,
                Err(error) => {
                    self.cleanup_error = Some(error.to_string());
                    false
                }
            };
            drop(guard);
            // Only this local clone and the runtime's owner may remain. The
            // server and SCK setup owners must be gone before the old allocation
            // is destroyed and the one-session reservation can be reused.
            if ready && Arc::strong_count(&producer) == 2 {
                drop(self.producer.take());
                drop(producer);
                self.drained = true;
                self.cleanup_error = None;
            } else if started.elapsed() >= DRAIN_TIMEOUT && self.cleanup_error.is_none() {
                self.cleanup_error =
                    Some("native shutdown needs recovery: mappings, setup owners or GPU writes have not retired".to_owned());
            }
        }
    }
}

struct NativeReady {
    producer: SharedProducer,
    width: u32,
    height: u32,
}

struct NativeRegistryPublisher {
    state: Mutex<NativeRuntime>,
    ready_tx: Mutex<Option<tokio::sync::oneshot::Sender<Result<NativeReady, String>>>>,
}

impl NativeRegistryPublisher {
    fn new(ready_tx: tokio::sync::oneshot::Sender<Result<NativeReady, String>>) -> Self {
        Self {
            state: Mutex::new(NativeRuntime::default()),
            ready_tx: Mutex::new(Some(ready_tx)),
        }
    }

    fn finish_start(&self, result: Result<NativeReady, String>) {
        if let Some(tx) = self.ready_tx.lock().expect("native startup poisoned").take() {
            let _ = tx.send(result);
        }
    }

    fn maintenance(self: &Arc<Self>) -> std::io::Result<()> {
        let publisher = Arc::clone(self);
        // Own retirement independently of the registry and async runtime. A
        // dropped session closes acquisition but cannot abort unresolved GPU use.
        std::thread::Builder::new()
            .name("native-capture-retirement".to_owned())
            .spawn(move || {
                loop {
                    // Consumer wakeups use Jackstay's event observers. This
                    // coarse host tick handles idle allocation retry and status.
                    std::thread::sleep(Duration::from_millis(100));
                    let mut state = publisher.state.lock().expect("native runtime poisoned");
                    state.maintain();
                    if state.drained {
                        break;
                    }
                }
            })
            .map(|_| ())
    }
}

impl NativeVideoFramePublisher for NativeRegistryPublisher {
    fn publish_native_frame(&self, frame: NativeCapturedFrame) {
        let mut state = self.state.lock().expect("native runtime poisoned");
        if state.closing.is_some() || state.error.is_some() {
            return;
        }
        let params = NativeStreamParams {
            width: frame.width,
            height: frame.height,
            pixel_format: PixelFormat::Bgra8Unorm,
            color_space: jackstay::model::ColorSpace::Srgb,
            clock_domain: jackstay::model::ClockDomain::MediaTime,
            modifier: 0,
        };
        if state.producer.is_none() {
            let result = MacosFrameBackend::new()
                .map_err(|error| error.to_string())
                .and_then(|backend| NativeArenaProducer::new(backend, params.clone(), arena_config()).map_err(|error| error.to_string()));
            match result {
                Ok(producer) => {
                    state.producer = Some(Arc::new(Mutex::new(producer)));
                    state.params = Some(params.clone());
                }
                Err(error) => {
                    state.error = Some(error.clone());
                    self.finish_start(Err(error));
                    return;
                }
            }
        }
        state.maintain();
        if state.error.is_some() {
            return;
        }
        let producer = Arc::clone(state.producer.as_ref().expect("native producer created"));
        let mut guard = producer.lock().expect("native producer poisoned");
        if state.pending.is_none() && state.params.as_ref() != Some(&params) {
            match guard.reconfigure(params.clone()) {
                Ok(ReconfigurationStatus::Ready { .. }) => {
                    state.params = Some(params);
                    state.pause = None;
                }
                Ok(ReconfigurationStatus::PausedCapacity { requested, available }) => {
                    state.pending = Some(params);
                    state.pause = Some((requested, available));
                }
                Err(error) => {
                    state.error = Some(error.to_string());
                    guard.stop();
                    return;
                }
            }
        }
        match guard.publish(&MacosCapturedFrame { surface: frame.surface }, frame.timestamp_ns) {
            Ok(PublishOutcome::Published { .. }) => {
                state.published = state.published.saturating_add(1);
                self.finish_start(Ok(NativeReady {
                    producer: Arc::clone(&producer),
                    width: frame.width,
                    height: frame.height,
                }));
            }
            Ok(PublishOutcome::Dropped) => {
                state.dropped = state.dropped.saturating_add(1);
            }
            Err(error) => {
                state.error = Some(error.to_string());
                self.finish_start(Err(error.to_string()));
                guard.stop();
            }
        }
    }

    fn capture_error(&self, message: &str) {
        let mut state = self.state.lock().expect("native runtime poisoned");
        state.error = Some(message.to_owned());
        if let Some(producer) = &state.producer {
            producer.lock().expect("native producer poisoned").stop();
        }
        self.finish_start(Err(message.to_owned()));
    }
}

fn native_error(error: impl std::fmt::Display) -> PortholeError {
    PortholeError::new(ErrorCode::InternalError, error.to_string())
}

/// Holds the `native_session_starting` reservation across the async startup.
/// Resets it on drop (any error return) unless the session committed, so a
/// failed start never wedges the one-session limit.
struct StartReservation<'a> {
    registry: &'a CaptureRegistry,
    active: bool,
    session_id: Option<String>,
}

impl Drop for StartReservation<'_> {
    fn drop(&mut self) {
        if self.active {
            if let Some(id) = &self.session_id {
                self.registry.remove_session(id);
            }
            if let Ok(mut inner) = self.registry.inner.lock() {
                inner.native_session_starting = false;
            }
        }
    }
}

/// Create a native capture session: start SCK native capture, build the
/// producer from the first frame, mint a per-session attach secret, and host
/// the named XPC attach server bound to that producer.
pub(super) async fn create(
    registry: &CaptureRegistry,
    surface: SurfaceInfo,
    owner_agent_id: AgentId,
) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
    // One native session at a time (see module note). Check-and-reserve in a
    // single locked step so two concurrent creates can't both pass — the
    // reservation is what protects portholed's single launchd mach name.
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
        if !inner.native_holds.is_empty() {
            return Err(CaptureRegistryError::Capture(inner.native_holds.values().next().map_or_else(
                || "a native capture session is starting".to_owned(),
                |hold| {
                    let snapshot = hold.snapshot();
                    format!("native session {}: {}", snapshot.status, snapshot.message.unwrap_or_default())
                },
            )));
        }
        inner.native_session_starting = true;
    }
    // From here, every early return must release the reservation; the guard
    // does it on drop unless we commit on success.
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
            lifecycle: CaptureSessionLifecycle::Starting,
            width: 0,
            height: 0,
            stride: 0,
            pixel_format: PixelFormat::Bgra8Unorm,
            cpu: None,
            capture_task: None,
            startup_cancel: None,
        },
    );

    reservation.session_id = Some(session_id.clone());
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let publisher = Arc::new(NativeRegistryPublisher::new(ready_tx));
    // Install the teardown owner before the first await. Cancellation and
    // startup errors must keep already-submitted GPU work charged too.
    let native_info = NativeCaptureInfo {
        transport_kind: NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
        endpoint: MACOS_NATIVE_ATTACH_MACH_SERVICE.to_string(),
        attach_token: format!("ptas_{}", Uuid::new_v4().simple()),
    };
    registry
        .inner
        .lock()
        .map_err(|_| CaptureRegistryError::Poisoned)?
        .native_holds
        .insert(
            session_id.clone(),
            NativeSessionHold::new(native_info.clone(), Arc::clone(&publisher))?,
        );
    let adapter = MacOsAdapter::new();
    let stream = match start_native_window_capture(&adapter, &surface, publisher.clone()).await {
        Ok(stream) => stream,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_porthole(error));
        }
    };
    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let hold = inner.native_holds.get_mut(&session_id).expect("startup hold remains reserved");
        // A close may have arrived while SCK started on its blocking thread.
        if hold.publisher.state.lock().expect("native runtime poisoned").closing.is_some() {
            drop(inner);
            drop(stream);
            return Err(CaptureRegistryError::Closed {
                session_id,
                message: "native capture session closed during startup".to_owned(),
            });
        }
        hold.stream = Some(stream);
    }

    let ready = match tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx).await {
        Ok(Ok(Ok(ready))) => ready,
        Ok(Ok(Err(message))) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::Capture(message));
        }
        _ => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::Capture("native capture produced no first frame".to_string()));
        }
    };

    // Per-session attach secret: a capability the descriptor hands the viewer
    // over the authenticated control socket. The daemon stores only agent
    // token hashes, so it cannot reuse the agent token as the bearer.
    let server = match XpcArenaServer::start_named(
        MACOS_NATIVE_ATTACH_MACH_SERVICE,
        Some(native_info.attach_token.clone()),
        Arc::clone(&ready.producer),
    ) {
        Ok(server) => server,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_porthole(native_error(error)));
        }
    };

    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let Some(session) = inner.sessions.get_mut(&session_id) else {
            return Err(CaptureRegistryError::Closed {
                session_id,
                message: "native capture session closed during startup".to_string(),
            });
        };
        if session.lifecycle != CaptureSessionLifecycle::Starting {
            return Err(CaptureRegistryError::Closed {
                session_id,
                message: "native capture session closed during startup".to_owned(),
            });
        }
        session.lifecycle = CaptureSessionLifecycle::Ready;
        session.width = ready.width;
        session.height = ready.height;
        session.pixel_format = PixelFormat::Bgra8Unorm;
        inner
            .native_holds
            .get_mut(&session_id)
            .expect("startup hold remains reserved")
            .server = Some(server);
        inner.native_session_starting = false;
    }
    // Committed: the hold now represents the session; don't let the guard
    // clear the (already-cleared) flag.
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
mod tests {
    use super::*;

    fn test_hold(publisher: Arc<NativeRegistryPublisher>) -> NativeSessionHold {
        NativeSessionHold::new(
            NativeCaptureInfo {
                transport_kind: NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
                endpoint: MACOS_NATIVE_ATTACH_MACH_SERVICE.to_owned(),
                attach_token: "test-only".to_owned(),
            },
            publisher,
        )
        .unwrap()
    }

    #[test]
    fn session_teardown_finishes_after_async_runtime_is_gone() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let publisher = Arc::new(NativeRegistryPublisher::new(tx));
        runtime.block_on(async {
            let hold = test_hold(Arc::clone(&publisher));
            drop(hold);
        });
        drop(runtime);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !publisher.state.lock().unwrap().drained {
            assert!(Instant::now() < deadline, "session teardown lost its cleanup owner");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    #[ignore = "requires a Metal device; submits an isolated GPU write behind a test gate"]
    fn dropped_session_retains_gpu_owner_until_write_completion() {
        use jackstay::{
            acquisition::arena::AcquireOutcome,
            model::{ClockDomain, ColorSpace},
            native::macos::{ConsumerFence, IoSurface},
        };

        let backend = MacosFrameBackend::new().unwrap();
        let gate = ConsumerFence::new(backend.metal()).unwrap();
        struct OpenOnDrop<'a>(&'a ConsumerFence);
        impl Drop for OpenOnDrop<'_> {
            fn drop(&mut self) {
                self.0.signal_cpu(1);
            }
        }
        let _open_on_unwind = OpenOnDrop(&gate);
        backend.metal().enqueue_wait(&gate, 1).unwrap();
        let params = NativeStreamParams {
            width: 16,
            height: 16,
            pixel_format: PixelFormat::Bgra8Unorm,
            color_space: ColorSpace::Srgb,
            clock_domain: ClockDomain::HostTime,
            modifier: 0,
        };
        let mut producer = NativeArenaProducer::new(backend, params.clone(), arena_config()).unwrap();
        let consumer = producer.attach(1).unwrap().into_consumer().unwrap();
        let surface = IoSurface::allocate(16, 16, PixelFormat::Bgra8Unorm).unwrap();
        surface.write_pixels(&vec![37; 16 * 16 * 4]).unwrap();
        let source = MacosCapturedFrame { surface };
        assert!(matches!(producer.publish(&source, 1).unwrap(), PublishOutcome::Published { .. }));
        let AcquireOutcome::Frame(held) = consumer.acquire_latest(0).unwrap() else {
            panic!("missing frame")
        };
        let producer = Arc::new(Mutex::new(producer));
        let retained = Arc::downgrade(&producer);
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let publisher = Arc::new(NativeRegistryPublisher::new(tx));
        {
            let mut state = publisher.state.lock().unwrap();
            state.producer = Some(producer);
            state.params = Some(params);
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async { drop(test_hold(Arc::clone(&publisher))) });
        drop(runtime);
        assert!(matches!(consumer.acquire_latest(0).unwrap(), AcquireOutcome::Closed));
        // No consumer GPU use was submitted. Releasing these owners does not
        // establish completion of the producer's gated write.
        drop(held);
        drop(consumer);
        std::thread::sleep(Duration::from_millis(250));
        assert!(
            retained.upgrade().is_some(),
            "teardown discarded the producer before GPU completion"
        );
        assert!(!publisher.state.lock().unwrap().drained);
        gate.signal_cpu(1);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !publisher.state.lock().unwrap().drained {
            assert!(
                Instant::now() < deadline,
                "completed GPU write did not drain after runtime teardown"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(retained.upgrade().is_none(), "drain retained the producer owner");
    }

    #[tokio::test]
    async fn capture_failure_reaches_the_startup_caller_without_waiting_for_a_frame() {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let publisher = NativeRegistryPublisher::new(tx);
        publisher.capture_error("capture stopped by the system");
        let result = rx.await.unwrap();
        assert!(matches!(result, Err(message) if message == "capture stopped by the system"));
        assert_eq!(publisher.state.lock().unwrap().snapshot().status, "failed");
    }

    #[tokio::test]
    async fn cancelled_startup_keeps_a_closing_owner_until_idle_maintenance_drains_it() {
        let registry = CaptureRegistry::disabled();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let publisher = Arc::new(NativeRegistryPublisher::new(tx));
        let id = "cancelled-startup".to_owned();
        {
            let mut inner = registry.inner.lock().unwrap();
            inner.native_session_starting = true;
            inner.native_holds.insert(id.clone(), test_hold(Arc::clone(&publisher)));
        }
        drop(StartReservation {
            registry: &registry,
            active: true,
            session_id: Some(id.clone()),
        });
        {
            let inner = registry.inner.lock().unwrap();
            assert!(!inner.native_session_starting);
            let hold = inner.native_holds.get(&id).expect("cancellation discarded the teardown owner");
            assert_eq!(hold.snapshot().status, "draining");
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if publisher.state.lock().unwrap().drained {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("idle cleanup did not complete");
        assert_eq!(registry.inner.lock().unwrap().native_holds[&id].snapshot().status, "closed");
    }
}
