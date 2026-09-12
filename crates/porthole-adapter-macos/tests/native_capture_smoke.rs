#![cfg(target_os = "macos")]

//! Live SCK → native acquisition arena → XPC → leased IOSurface validation.
//! The consumer deliberately holds each frame while capture continues, then
//! verifies its pixels again before release. No synthetic capture source.
//!
//! Requires a real macOS desktop session with Screen Recording granted (per
//! AGENTS.md). The XPC listener here is anonymous (same-process rendezvous);
//! the launchd-named path is the same code with `start_named` and is owned
//! by portholed once native tracks join the session API.

use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Sender, channel},
    },
    time::Duration,
};

use jackstay::{
    acquisition::arena::{AcquireOutcome, ArenaConfig, Cancellation, PublishOutcome, WaitInterest, WaitOutcome},
    model::{ClockDomain, ColorSpace, PixelFormat},
    native::{
        NativeStreamParams,
        arena::NativeArenaProducer,
        macos::{
            ConsumerFence, IoSurface, MacosCapturedFrame, MacosFrameBackend, MetalContext, SharedEventHandle,
            xpc::arena::{XpcArenaClient, XpcArenaServer},
        },
    },
};
use porthole_adapter_macos::{
    MacOsAdapter,
    sck_native::{NativeCapturedFrame, NativeVideoFramePublisher, start_native_window_capture},
};
use porthole_core::{
    adapter::{Adapter, ProcessLaunchSpec, RequireConfidence},
    surface::{PlatformSurfaceRef, SurfaceId, SurfaceInfo},
};

type SharedProducer = Arc<Mutex<NativeArenaProducer<MacosFrameBackend>>>;

/// Builds the producer from the first frame's dimensions, then pumps every
/// frame into it. The stream's true dimensions are only known at the first
/// callback. This fixture tests a fixed-size source; session reconfiguration
/// is handled by portholed's native registry publisher.
struct SmokePublisher {
    producer: Mutex<Option<SharedProducer>>,
    ready: Sender<SharedProducer>,
    errors: Mutex<Vec<String>>,
}

impl NativeVideoFramePublisher for SmokePublisher {
    fn publish_native_frame(&self, frame: NativeCapturedFrame) {
        let mut slot = self.producer.lock().unwrap();
        let producer = match slot.as_ref() {
            Some(producer) => Arc::clone(producer),
            None => {
                let backend = MacosFrameBackend::new().expect("Metal device");
                let params = NativeStreamParams {
                    width: frame.width,
                    height: frame.height,
                    pixel_format: PixelFormat::Bgra8Unorm,
                    color_space: ColorSpace::Srgb,
                    clock_domain: ClockDomain::MediaTime,
                    modifier: 0,
                };
                let producer = Arc::new(Mutex::new(
                    NativeArenaProducer::new(
                        backend,
                        params,
                        ArenaConfig {
                            resource_capacity: 8,
                            retained_history: 2,
                            producer_reserve: 1,
                            payload_capacity: 0,
                            memory_budget: 512 * 1024 * 1024,
                            max_incarnations: 4,
                            drain_timeout: Duration::from_secs(5),
                        },
                    )
                    .expect("create native producer"),
                ));
                *slot = Some(Arc::clone(&producer));
                let _ = self.ready.send(Arc::clone(&producer));
                producer
            }
        };
        let captured = MacosCapturedFrame { surface: frame.surface };
        match producer.lock().unwrap().publish(&captured, frame.timestamp_ns) {
            Ok(PublishOutcome::Published { .. } | PublishOutcome::Dropped) => {}
            Err(error) => self.errors.lock().unwrap().push(error.to_string()),
        }
    }

    fn capture_error(&self, message: &str) {
        self.errors.lock().unwrap().push(message.to_owned());
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session with Screen Recording permission"]
async fn sck_iosurface_stays_immutable_through_xpc_acquisition_and_delayed_release() {
    let adapter = MacOsAdapter::new();
    // PORTHOLE_SMOKE_CG_WINDOW_ID targets an existing window directly,
    // bypassing launch correlation (useful when the launch-tag mechanism is
    // unavailable, or to capture an arbitrary window). Otherwise launch the
    // app named by PORTHOLE_SMOKE_APP (TextEdit by default; it must not
    // already be running for correlation to tag a fresh window).
    let surface = match std::env::var("PORTHOLE_SMOKE_CG_WINDOW_ID") {
        Ok(id) => {
            let cg_window_id: u32 = id.parse().expect("PORTHOLE_SMOKE_CG_WINDOW_ID must be a u32");
            let mut surface = SurfaceInfo::window(SurfaceId::new(), 0);
            surface.platform_ref = Some(PlatformSurfaceRef::macos(cg_window_id));
            surface
        }
        Err(_) => {
            let app = std::env::var("PORTHOLE_SMOKE_APP").unwrap_or_else(|_| "/System/Applications/TextEdit.app".to_string());
            let spec = ProcessLaunchSpec {
                app,
                args: vec![],
                cwd: None,
                env: vec![],
                timeout: Duration::from_secs(10),
                require_confidence: RequireConfidence::Strong,
                require_fresh_surface: false,
                force_place: false,
            };
            adapter.launch_process(&spec).await.expect("launch smoke app").surface
        }
    };

    let (ready_tx, ready_rx) = channel();
    let publisher = Arc::new(SmokePublisher {
        producer: Mutex::new(None),
        ready: ready_tx,
        errors: Mutex::new(Vec::new()),
    });
    let stream = start_native_window_capture(&adapter, &surface, publisher.clone())
        .await
        .expect("start native SCK capture");

    // The first frame builds the producer with the stream's real dimensions.
    let producer = ready_rx.recv_timeout(Duration::from_secs(10)).expect("first SCK frame");

    // The process-bound grant transfers the arena maps, surfaces and readiness.
    let (server, listener_endpoint) = XpcArenaServer::start_anonymous(Some("pta_agent.smoke".to_owned()), Arc::clone(&producer)).unwrap();
    let mut client = XpcArenaClient::connect_endpoint(&listener_endpoint).unwrap();
    client.authorize("pta_agent.smoke").unwrap();
    let mut consumer = client.attach(1).unwrap();
    let metal = MetalContext::new().unwrap();

    // Steady state: follow live frames via shared memory only. Each frame's
    // pixels must be ready once the fence reaches its value.
    let mut last_cursor = 0;
    let mut observed = 0;
    let frames: u32 = std::env::var("PORTHOLE_SMOKE_FRAMES").map_or(3, |value| value.parse().expect("PORTHOLE_SMOKE_FRAMES"));
    assert!(frames > 0);
    let hold_ms: u64 = std::env::var("PORTHOLE_SMOKE_HOLD_MS").map_or(250, |value| value.parse().expect("PORTHOLE_SMOKE_HOLD_MS"));
    let duration = Duration::from_millis(hold_ms).saturating_mul(frames) + Duration::from_secs(15);
    let deadline = std::time::Instant::now() + duration;
    let cancellation = Cancellation::new().unwrap();
    while observed < frames {
        assert!(
            publisher.errors.lock().unwrap().is_empty(),
            "capture errors: {:?}",
            publisher.errors.lock().unwrap()
        );
        assert!(std::time::Instant::now() < deadline, "saw only {observed} live frames");
        let before = consumer.events();
        let held = match consumer.acquire_latest(last_cursor).unwrap() {
            AcquireOutcome::Frame(held) => held,
            AcquireOutcome::Empty | AcquireOutcome::Miss { .. } => {
                assert!(
                    matches!(
                        consumer
                            .wait(before, WaitInterest::DATA, &cancellation, Some(Duration::from_secs(5)))
                            .unwrap(),
                        WaitOutcome::Changed(_)
                    ),
                    "live source stopped publishing"
                );
                continue;
            }
            other => panic!("unexpected acquisition outcome: {other:?}"),
        };
        let entry = *held.descriptor();
        let native = held.native_resources::<IoSurface, SharedEventHandle>().unwrap();
        let fence = ConsumerFence::from_handle(&metal, native.sync_handle).unwrap();
        assert!(
            fence.wait(entry.fence_value, 5_000),
            "fence not signalled for sequence {}",
            entry.sequence
        );

        let surface = native.surface;
        let mut pixels = vec![0u8; entry.width as usize * entry.height as usize * 4];
        surface.read_pixels(&mut pixels).unwrap();
        assert!(
            pixels.iter().any(|byte| *byte != 0),
            "sequence {}: transferred surface is all zeroes",
            entry.sequence
        );
        tokio::time::sleep(Duration::from_millis(hold_ms)).await;
        let mut after = vec![0; pixels.len()];
        surface.read_pixels(&mut after).unwrap();
        assert_eq!(after, pixels, "held live pixels changed before release");
        assert_eq!(held.descriptor(), &entry);
        last_cursor = held.cursor();
        drop(fence);
        drop(held);
        observed += 1;
    }
    drop(consumer);
    drop(client);
    drop(server);
    drop(stream);
    producer.lock().unwrap().stop();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !producer.lock().unwrap().poll_shutdown_ready().unwrap() {
        assert!(std::time::Instant::now() < deadline, "live acquisition teardown did not drain");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        publisher.errors.lock().unwrap().is_empty(),
        "capture errors: {:?}",
        publisher.errors.lock().unwrap()
    );
}
