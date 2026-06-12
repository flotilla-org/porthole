#![cfg(target_os = "macos")]

//! Live end-to-end validation for #84: a real SCK window stream, IOSurface
//! extraction, Metal blit staging into the producer pool, ring publication,
//! XPC attach with the grant crossing a real NSXPCConnection, and a consumer
//! that maps the ring from the transferred fd, waits the shared-event fence,
//! and reads pixels from a transferred surface.
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

use capture_transfer::{
    control_page::VideoTrackControlPage,
    model::{ClockDomain, ColorSpace, PixelFormat},
    native::{
        NativeStreamParams, NativeTrackProducer, PoolExhaustionPolicy,
        attach::AttachEndpoint,
        macos::{
            ConsumerFence, MacosCapturedFrame, MacosFrameBackend, MetalContext,
            xpc::{XpcAttachClient, XpcAttachServer},
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

type SharedProducer = Arc<Mutex<NativeTrackProducer<MacosFrameBackend>>>;

/// Builds the producer from the first frame's dimensions, then pumps every
/// frame into it. The stream's true dimensions are only known at the first
/// callback; a resize after that would need a new pool (deferred, like the
/// CPU path).
struct SmokePublisher {
    producer: Mutex<Option<SharedProducer>>,
    ready: Sender<SharedProducer>,
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
                    NativeTrackProducer::new(backend, params, 4, 6, PoolExhaustionPolicy::DropFrame).expect("create native producer"),
                ));
                *slot = Some(Arc::clone(&producer));
                let _ = self.ready.send(Arc::clone(&producer));
                producer
            }
        };
        let captured = MacosCapturedFrame { surface: frame.surface };
        if let Err(error) = producer.lock().unwrap().publish(&captured, frame.timestamp_ns) {
            // A resize mid-stream lands here (new pool is a later slice);
            // anything else is a real failure worth seeing in --nocapture.
            eprintln!("native publish failed: {error}");
        }
    }

    fn capture_error(&self, message: &str) {
        eprintln!("native capture error: {message}");
    }
}

#[tokio::test]
#[ignore = "requires a real macOS desktop session with Screen Recording permission"]
async fn sck_iosurface_flows_through_ring_and_xpc_attach_to_a_fenced_consumer() {
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
    });
    let _stream = start_native_window_capture(&adapter, &surface, publisher)
        .await
        .expect("start native SCK capture");

    // The first frame builds the producer with the stream's real dimensions.
    let producer = ready_rx.recv_timeout(Duration::from_secs(10)).expect("first SCK frame");

    // Attach over a real XPC connection: ring fd, surfaces, and the shared
    // event handle all cross as the one-time grant.
    let endpoint = AttachEndpoint::new(Some("pta_agent.smoke".to_string()));
    let (_server, listener_endpoint) = XpcAttachServer::start_anonymous(endpoint, Arc::clone(&producer)).unwrap();
    let client = XpcAttachClient::connect_endpoint(&listener_endpoint).unwrap();
    client.authorize("pta_agent.smoke").unwrap();
    let grant = client.attach(1).unwrap();

    let ring = VideoTrackControlPage::map_read_only(grant.ring_fd, grant.ring_map_len as usize).unwrap();
    let metal = MetalContext::new().unwrap();
    let fence = ConsumerFence::from_handle(&metal, &grant.sync_handle).unwrap();

    // Steady state: follow live frames via shared memory only. Each frame's
    // pixels must be ready once the fence reaches its value.
    let mut last_sequence = 0;
    let mut observed = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while observed < 3 {
        assert!(std::time::Instant::now() < deadline, "saw only {observed} live frames in 15s");
        let Some(entry) = ring.read_latest_lossy_entry().unwrap() else {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        };
        if entry.sequence == last_sequence {
            tokio::time::sleep(Duration::from_millis(20)).await;
            continue;
        }
        assert_eq!(entry.pool_id, grant.pool_id);
        assert!(
            fence.wait(entry.fence_value, 5_000),
            "fence not signalled for sequence {}",
            entry.sequence
        );

        let surface = &grant.surface_handles[entry.slot_id as usize];
        let mut pixels = vec![0u8; entry.width as usize * entry.height as usize * 4];
        surface.read_pixels(&mut pixels).unwrap();
        assert!(
            pixels.iter().any(|byte| *byte != 0),
            "sequence {}: transferred surface is all zeroes",
            entry.sequence
        );
        last_sequence = entry.sequence;
        observed += 1;
    }
}
