//! macOS native (IOSurface/Metal) capture sessions (#85).
//!
//! Unlike the CPU path — which copies pixels into a shared-memory pool a
//! consumer reads over the fd socket — a native session drives a
//! [`NativeTrackProducer`] from ScreenCaptureKit's backing IOSurfaces and
//! hosts an [`XpcAttachServer`] on portholed's launchd `MachServices` name.
//! Consumers attach over XPC, receive the surfaces + shared-event handle
//! once, and then read frames straight from shared memory (ADR-0007).
//!
//! Scope (#85): exactly one native session at a time. launchd permits a
//! single listener per mach-service name, so the server is a daemon
//! singleton bound to the one active native session; a second request while
//! one is live is rejected. Multiplexing many native sessions behind one
//! broker is a follow-up.

use std::sync::{Arc, Mutex};

use capture_transfer::{
    model::{PixelFormat, SourceDesc, SourceKind, TrackDesc, VideoTrackDesc},
    native::{
        NativeStreamParams, NativeTrackProducer, PoolExhaustionPolicy,
        attach::AttachEndpoint,
        macos::{MacosCapturedFrame, MacosFrameBackend, xpc::XpcAttachServer},
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

type SharedProducer = Arc<Mutex<NativeTrackProducer<MacosFrameBackend>>>;

/// Keeps a native session's runtime alive for the session's lifetime. Drop
/// order (declaration order) stops the XPC server first (no new attaches),
/// then the SCK stream (no new frames), then the producer (frees pool/fence).
pub(super) struct NativeSessionHold {
    pub native_info: NativeCaptureInfo,
    _server: XpcAttachServer,
    _stream: NativeSckCaptureStream,
    _producer: SharedProducer,
}

impl std::fmt::Debug for NativeSessionHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSessionHold")
            .field("native_info", &self.native_info)
            .finish_non_exhaustive()
    }
}

/// First-frame readiness: the producer (built once the stream's real
/// dimensions are known) and those dimensions.
struct NativeReady {
    producer: SharedProducer,
    width: u32,
    height: u32,
}

/// Builds the producer from the first frame's dimensions, then pumps every
/// frame into it. A resize after that needs a new pool/generation (deferred,
/// like the CPU path); `publish` errors on a dimension mismatch and the frame
/// is dropped with a log rather than crashing the daemon.
struct NativeRegistryPublisher {
    producer: Mutex<Option<SharedProducer>>,
    ready_tx: Mutex<Option<tokio::sync::oneshot::Sender<NativeReady>>>,
}

impl NativeRegistryPublisher {
    fn new(ready_tx: tokio::sync::oneshot::Sender<NativeReady>) -> Self {
        Self {
            producer: Mutex::new(None),
            ready_tx: Mutex::new(Some(ready_tx)),
        }
    }

    fn build_producer(&self, frame: &NativeCapturedFrame) -> Result<SharedProducer, PortholeError> {
        let backend = MacosFrameBackend::new().map_err(native_error)?;
        let params = NativeStreamParams {
            width: frame.width,
            height: frame.height,
            pixel_format: PixelFormat::Bgra8Unorm,
            color_space: capture_transfer::model::ColorSpace::Srgb,
            clock_domain: capture_transfer::model::ClockDomain::MediaTime,
            modifier: 0,
        };
        // Ring 4 / pool 8: latest-wins capture, a comfortable margin over the
        // live window so a holding consumer never stalls the producer.
        let producer = NativeTrackProducer::new(backend, params, 4, 8, PoolExhaustionPolicy::DropFrame).map_err(native_error)?;
        Ok(Arc::new(Mutex::new(producer)))
    }
}

impl NativeVideoFramePublisher for NativeRegistryPublisher {
    fn publish_native_frame(&self, frame: NativeCapturedFrame) {
        let producer = {
            let mut slot = self.producer.lock().expect("native publisher producer poisoned");
            match slot.as_ref() {
                Some(producer) => Arc::clone(producer),
                None => match self.build_producer(&frame) {
                    Ok(producer) => {
                        *slot = Some(Arc::clone(&producer));
                        if let Some(tx) = self.ready_tx.lock().expect("native publisher ready_tx poisoned").take() {
                            let _ = tx.send(NativeReady {
                                producer: Arc::clone(&producer),
                                width: frame.width,
                                height: frame.height,
                            });
                        }
                        producer
                    }
                    Err(error) => {
                        tracing::error!(%error, "failed to build native producer from first SCK frame");
                        return;
                    }
                },
            }
        };
        let captured = MacosCapturedFrame { surface: frame.surface };
        if let Err(error) = producer
            .lock()
            .expect("native producer poisoned")
            .publish(&captured, frame.timestamp_ns)
        {
            // A mid-stream resize lands here (new pool is a later slice);
            // drop the frame rather than tearing down the daemon.
            tracing::warn!(%error, "native frame publish failed");
        }
    }

    fn capture_error(&self, message: &str) {
        tracing::warn!(message, "native capture stream error");
    }
}

fn native_error(error: capture_transfer::CaptureTransferError) -> PortholeError {
    PortholeError::new(ErrorCode::InternalError, error.to_string())
}

/// Holds the `native_session_starting` reservation across the async startup.
/// Resets it on drop (any error return) unless the session committed, so a
/// failed start never wedges the one-session limit.
struct StartReservation<'a> {
    registry: &'a CaptureRegistry,
    active: bool,
}

impl Drop for StartReservation<'_> {
    fn drop(&mut self) {
        if self.active
            && let Ok(mut inner) = self.registry.inner.lock()
        {
            inner.native_session_starting = false;
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
        if !inner.native_holds.is_empty() || inner.native_session_starting {
            return Err(CaptureRegistryError::Capture(
                "a native capture session is already active (one at a time on this build)".to_string(),
            ));
        }
        inner.native_session_starting = true;
    }
    // From here, every early return must release the reservation; the guard
    // does it on drop unless we commit on success.
    let mut reservation = StartReservation { registry, active: true };

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
            video: capture_transfer::video::VideoSlotManager::new_reusable_pool(1),
            capture_task: None,
            startup_cancel: None,
        },
    );

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let publisher = Arc::new(NativeRegistryPublisher::new(ready_tx));
    let adapter = MacOsAdapter::new();
    let stream = match start_native_window_capture(&adapter, &surface, publisher).await {
        Ok(stream) => stream,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_porthole(error));
        }
    };

    let ready = match tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx).await {
        Ok(Ok(ready)) => ready,
        _ => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::Capture("native capture produced no first frame".to_string()));
        }
    };

    // Per-session attach secret: a capability the descriptor hands the viewer
    // over the authenticated control socket. The daemon stores only agent
    // token hashes, so it cannot reuse the agent token as the bearer.
    let attach_token = format!("ptas_{}", Uuid::new_v4().simple());
    let endpoint = AttachEndpoint::new(Some(attach_token.clone()));
    let server = match XpcAttachServer::start_named(MACOS_NATIVE_ATTACH_MACH_SERVICE, endpoint, Arc::clone(&ready.producer)) {
        Ok(server) => server,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_capture(error));
        }
    };

    let native_info = NativeCaptureInfo {
        transport_kind: NATIVE_ATTACH_TRANSPORT_MACOS_XPC,
        endpoint: MACOS_NATIVE_ATTACH_MACH_SERVICE.to_string(),
        attach_token,
    };

    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let Some(session) = inner.sessions.get_mut(&session_id) else {
            return Err(CaptureRegistryError::Closed {
                session_id,
                message: "native capture session closed during startup".to_string(),
            });
        };
        session.lifecycle = CaptureSessionLifecycle::Ready;
        session.width = ready.width;
        session.height = ready.height;
        session.pixel_format = PixelFormat::Bgra8Unorm;
        inner.native_holds.insert(
            session_id.clone(),
            NativeSessionHold {
                native_info: native_info.clone(),
                _server: server,
                _stream: stream,
                _producer: ready.producer,
            },
        );
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
