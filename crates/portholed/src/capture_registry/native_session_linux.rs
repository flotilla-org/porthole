//! Linux native dmabuf/PipeWire capture sessions (ADR-0009).

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use capture_transfer::{
    model::{PixelFormat, SourceDesc, SourceKind, TrackDesc, VideoTrackDesc},
    native::{
        PoolExhaustionPolicy,
        attach::AttachEndpoint,
        linux::{
            LinuxAttachServer,
            pipewire::{
                PipeWireBufferReleaser, PipeWireNativeBackend, PipeWireNativeProducerHandle, PipeWireNativeProducerObserver,
                PipeWireStream, PipeWireStreamTarget, SharedPipeWireNativeProducer,
            },
        },
    },
    state::SessionState,
    video::VideoSlotManager,
};
use porthole_adapter_kwin::{
    KWinAdapter,
    screencast::{ScreenCastSession, ScreenCastStream},
};
use porthole_core::{ErrorCode, PortholeError, agent_policy::AgentId, surface::SurfaceInfo};
use porthole_protocol::capture_sessions::{CreateCaptureSessionResponse, NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET, NativeCaptureInfo};
use uuid::Uuid;

use super::{CaptureRegistry, CaptureRegistryError, CaptureSession, CaptureSessionLifecycle};

const PRODUCER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const PRODUCER_READY_POLL: Duration = Duration::from_millis(10);

/// Keeps a Linux native session's runtime alive for the session lifetime. Drop
/// order stops the attach server first, then the PipeWire stream, then the
/// portal session, then the producer.
pub(super) struct LinuxNativeSessionHold {
    pub native_info: NativeCaptureInfo,
    _server: LinuxAttachServer<PipeWireNativeBackend>,
    _stream: PipeWireStream,
    _screencast: ScreenCastSession,
    _producer: SharedPipeWireNativeProducer,
}

impl std::fmt::Debug for LinuxNativeSessionHold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxNativeSessionHold")
            .field("native_info", &self.native_info)
            .finish_non_exhaustive()
    }
}

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

pub(super) async fn create(
    registry: &CaptureRegistry,
    kwin_adapter: Arc<KWinAdapter>,
    surface: SurfaceInfo,
    owner_agent_id: AgentId,
) -> Result<CreateCaptureSessionResponse, CaptureRegistryError> {
    {
        let mut inner = registry.inner.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        if !inner.native_holds.is_empty() || inner.native_session_starting {
            return Err(CaptureRegistryError::Capture(
                "a native capture session is already active (one at a time on this build)".to_string(),
            ));
        }
        inner.native_session_starting = true;
    }
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
            video: VideoSlotManager::new_reusable_pool(1),
            capture_task: None,
            startup_cancel: None,
        },
    );

    let buffer_releaser = PipeWireBufferReleaser::new();
    let backend = match PipeWireNativeBackend::open_with_buffer_releaser(drm_render_node(), Some(buffer_releaser.clone())) {
        Ok(backend) => backend,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_capture(error));
        }
    };
    let screencast = match kwin_adapter.start_screencast_session().await {
        Ok(session) => session,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_porthole(error));
        }
    };
    let stream_info = match screencast.streams.first().cloned() {
        Some(stream) => stream,
        None => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_porthole(PortholeError::new(
                ErrorCode::InternalError,
                "ScreenCast portal returned no PipeWire streams",
            )));
        }
    };
    let (observer, producer_handle) = PipeWireNativeProducerObserver::new(backend, 1, PoolExhaustionPolicy::DropFrame);
    let stream = match PipeWireStream::open_remote_with_observer_and_releaser(
        &screencast.pipewire_remote,
        stream_target(&stream_info),
        Box::new(observer),
        buffer_releaser,
    ) {
        Ok(stream) => stream,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_capture(error));
        }
    };
    let producer = match wait_for_producer(&producer_handle).await {
        Ok(producer) => producer,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(error);
        }
    };

    let first_frame = {
        let producer = producer.lock().expect("linux native producer poisoned");
        let Some(cursor) = producer.control_page().latest_cursor() else {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::Capture(
                "native PipeWire stream produced no first frame".to_string(),
            ));
        };
        producer.control_page().read_entry_for_cursor(cursor).map_err(|error| {
            // Fallible under a fast producer: the ring can lap this
            // cursor between latest_cursor() above and the read.
            registry.remove_session(&session_id);
            CaptureRegistryError::Capture(error.to_string())
        })?
    };

    let attach_token = format!("ptas_{}", Uuid::new_v4().simple());
    let attach_path = attach_socket_path(&session_id);
    let endpoint = AttachEndpoint::new(Some(attach_token.clone()));
    let server = match LinuxAttachServer::start(&attach_path, endpoint, Arc::clone(&producer)) {
        Ok(server) => server,
        Err(error) => {
            registry.remove_session(&session_id);
            return Err(CaptureRegistryError::from_capture(error));
        }
    };
    let native_info = NativeCaptureInfo {
        transport_kind: NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET,
        endpoint: attach_path.to_string_lossy().to_string(),
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
        session.width = first_frame.width;
        session.height = first_frame.height;
        session.pixel_format = pixel_format_from_native(first_frame.pixel_format);
        inner.native_holds.insert(
            session_id.clone(),
            LinuxNativeSessionHold {
                native_info: native_info.clone(),
                _server: server,
                _stream: stream,
                _screencast: screencast,
                _producer: producer,
            },
        );
        inner.native_session_starting = false;
    }
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

async fn wait_for_producer(handle: &PipeWireNativeProducerHandle) -> Result<SharedPipeWireNativeProducer, CaptureRegistryError> {
    let deadline = Instant::now() + PRODUCER_READY_TIMEOUT;
    loop {
        if let Some(producer) = handle.producer() {
            return Ok(producer);
        }
        if let Some(error) = handle.last_error() {
            return Err(CaptureRegistryError::Capture(error));
        }
        if Instant::now() >= deadline {
            return Err(CaptureRegistryError::Capture(
                "native PipeWire stream produced no dmabuf-backed first frame".to_string(),
            ));
        }
        tokio::time::sleep(PRODUCER_READY_POLL).await;
    }
}

fn stream_target(stream: &ScreenCastStream) -> PipeWireStreamTarget {
    PipeWireStreamTarget {
        node_id: stream.node_id,
        object_serial: stream.pipewire_serial,
    }
}

fn drm_render_node() -> PathBuf {
    std::env::var_os("PORTHOLE_DRM_RENDER_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/dev/dri/renderD128"))
}

fn attach_socket_path(session_id: &str) -> PathBuf {
    crate::runtime::socket_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("capture-native-{session_id}.sock"))
}

fn pixel_format_from_native(pixel_format: u32) -> PixelFormat {
    match pixel_format {
        value if value == PixelFormat::Bgra8Unorm as u32 => PixelFormat::Bgra8Unorm,
        value if value == PixelFormat::Rgba8Unorm as u32 => PixelFormat::Rgba8Unorm,
        _ => PixelFormat::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::CString, ptr, sync::Arc};

    use capture_transfer::{
        ffi::{FT_STATUS_EMPTY, FT_STATUS_OK, FT_STATUS_TIMEOUT},
        ffi_native::{
            FT_NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET, FT_NATIVE_HANDLE_DMABUF, FT_NATIVE_RELEASE_NOW, FT_NATIVE_SYNC_DRM_SYNCOBJ_TIMELINE,
            FtNativeAttach, FtNativeAttachDescriptor, FtNativeFrame, FtNativeGrant, FtNativeRelease, ft_native_acquire_latest,
            ft_native_attach_connect, ft_native_attach_destroy, ft_native_attach_grant, ft_native_release_frame, ft_native_wait_frame,
        },
    };
    use porthole_adapter_kwin::{KWinAdapter, bridge::KWinBridge};
    use porthole_core::{SurfaceId, agent_policy::AgentId, surface::SurfaceInfo};

    use super::*;
    use crate::capture_registry::CaptureRegistry;

    struct LiveNativeSession {
        registry: CaptureRegistry,
        session_id: String,
        attach: *mut FtNativeAttach,
        grant: FtNativeGrant,
    }

    impl Drop for LiveNativeSession {
        fn drop(&mut self) {
            if !self.attach.is_null() {
                unsafe { ft_native_attach_destroy(self.attach) };
            }
            let _ = self.registry.close_session(&self.session_id);
        }
    }

    fn empty_frame() -> FtNativeFrame {
        FtNativeFrame {
            struct_size: std::mem::size_of::<FtNativeFrame>() as u32,
            flags: 0,
            lease_id: 0,
            cursor: 0,
            sequence: 0,
            timestamp_ns: 0,
            pool_id: 0,
            slot_id: 0,
            width: 0,
            height: 0,
            pixel_format: 0,
            producer_sync_id: 0,
            producer_sync_value: 0,
        }
    }

    fn release_now(attach: *mut FtNativeAttach, lease_id: u64) {
        let release = FtNativeRelease {
            struct_size: std::mem::size_of::<FtNativeRelease>() as u32,
            release_kind: FT_NATIVE_RELEASE_NOW,
            lease_id,
            release_sync_id: 0,
            release_value: 0,
        };
        assert_eq!(unsafe { ft_native_release_frame(attach, &release) }, FT_STATUS_OK);
    }

    async fn connect_live_native_session(test_name: &str, agent_id: &str) -> LiveNativeSession {
        let registry = CaptureRegistry::disabled();
        let kwin = Arc::new(KWinAdapter::new(KWinBridge::new()));
        let mut surface = SurfaceInfo::window(SurfaceId::new(), std::process::id());
        surface.title = Some(test_name.to_string());

        let response = create(&registry, kwin, surface, AgentId::from(agent_id))
            .await
            .expect("native KDE/PipeWire capture session should start");
        let native = response.native.expect("native capture info should be present");
        assert_eq!(native.transport_kind, FT_NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET);

        let endpoint = CString::new(native.endpoint).unwrap();
        let token = CString::new(native.attach_token).unwrap();
        let descriptor = FtNativeAttachDescriptor {
            struct_size: std::mem::size_of::<FtNativeAttachDescriptor>() as u32,
            transport_kind: FT_NATIVE_ATTACH_TRANSPORT_UNIX_SOCKET,
            requested_consumer_id: 0,
            endpoint: endpoint.as_ptr(),
            bearer_token: token.as_ptr(),
            flags: 0,
        };
        let mut attach: *mut FtNativeAttach = ptr::null_mut();
        assert_eq!(unsafe { ft_native_attach_connect(&descriptor, &mut attach) }, FT_STATUS_OK);
        assert!(!attach.is_null());

        let mut grant = FtNativeGrant {
            struct_size: std::mem::size_of::<FtNativeGrant>() as u32,
            pool_count: 0,
            consumer_id: 0,
            consumer_slot: 0,
            pools: ptr::null(),
            producer_sync: unsafe { std::mem::zeroed() },
        };
        assert_eq!(unsafe { ft_native_attach_grant(attach, &mut grant) }, FT_STATUS_OK);
        assert_eq!(grant.pool_count, 1);
        assert_eq!(grant.producer_sync.sync_kind, FT_NATIVE_SYNC_DRM_SYNCOBJ_TIMELINE);

        LiveNativeSession {
            registry,
            session_id: response.session_id,
            attach,
            grant,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires a real KDE/Wayland session, ScreenCast portal consent, PipeWire, and /dev/dri/renderD128"]
    async fn live_kde_pipewire_native_session_attaches_and_acquires_one_dmabuf_frame() {
        if std::env::var_os("PORTHOLE_LIVE_KDE_NATIVE_SMOKE").is_none() {
            eprintln!("set PORTHOLE_LIVE_KDE_NATIVE_SMOKE=1 to run the live KDE/PipeWire native capture smoke");
            return;
        }
        let session = connect_live_native_session("porthole linux native smoke", "agent_linux_native_smoke").await;

        let mut ready_cursor = 0;
        let wait_status = unsafe { ft_native_wait_frame(session.attach, 0, 1_000_000_000, &mut ready_cursor) };
        assert!(
            wait_status == FT_STATUS_OK || wait_status == FT_STATUS_TIMEOUT,
            "unexpected wait status {wait_status}"
        );

        let mut frame = empty_frame();
        assert_eq!(unsafe { ft_native_acquire_latest(session.attach, 0, &mut frame) }, FT_STATUS_OK);
        assert_ne!(frame.lease_id, 0);
        assert!(frame.width > 0);
        assert!(frame.height > 0);

        let pool = unsafe { *session.grant.pools };
        assert_eq!(pool.pool_id, frame.pool_id);
        assert_eq!(
            pool.surface_count, 4,
            "KWin should honor porthole's requested four-buffer PipeWire pool"
        );
        assert!(frame.slot_id < pool.surface_count);
        let surface = unsafe { *pool.surfaces.add(frame.slot_id as usize) };
        assert_eq!(surface.handle_kind, FT_NATIVE_HANDLE_DMABUF);
        assert!(surface.plane_count > 0);
        assert!(surface.planes[0].fd >= 0);

        release_now(session.attach, frame.lease_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires a real KDE/Wayland session, ScreenCast portal consent, PipeWire, and /dev/dri/renderD128"]
    async fn live_kde_pipewire_native_session_holds_pipewire_slot_until_lease_release() {
        if std::env::var_os("PORTHOLE_LIVE_KDE_NATIVE_SMOKE").is_none() {
            eprintln!("set PORTHOLE_LIVE_KDE_NATIVE_SMOKE=1 to run the live KDE/PipeWire native capture smoke");
            return;
        }
        let session = connect_live_native_session("porthole linux native lease_gate", "agent_linux_native_lease_gate").await;
        let pool = unsafe { *session.grant.pools };
        assert_eq!(
            pool.surface_count, 4,
            "KWin should honor porthole's requested four-buffer PipeWire pool"
        );

        let mut first = empty_frame();
        assert_eq!(
            unsafe { ft_native_wait_frame(session.attach, 0, 1_000_000_000, &mut first.cursor) },
            FT_STATUS_OK
        );
        assert_eq!(unsafe { ft_native_acquire_latest(session.attach, 0, &mut first) }, FT_STATUS_OK);
        let held_lease_id = first.lease_id;
        let held_slot_id = first.slot_id;
        let mut last_cursor = first.cursor;
        let mut observed_other_slot = false;

        let before_release_deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < before_release_deadline {
            let mut ready_cursor = 0;
            let wait_status = unsafe { ft_native_wait_frame(session.attach, last_cursor, 500_000_000, &mut ready_cursor) };
            if wait_status == FT_STATUS_TIMEOUT {
                continue;
            }
            assert_eq!(wait_status, FT_STATUS_OK, "unexpected wait status {wait_status}");

            let mut frame = empty_frame();
            let acquire_status = unsafe { ft_native_acquire_latest(session.attach, last_cursor, &mut frame) };
            if acquire_status == FT_STATUS_EMPTY {
                continue;
            }
            assert_eq!(acquire_status, FT_STATUS_OK, "unexpected acquire status {acquire_status}");
            assert_ne!(
                frame.slot_id, held_slot_id,
                "held PipeWire slot {held_slot_id} was republished before its native lease released"
            );
            observed_other_slot = true;
            last_cursor = frame.cursor;
            release_now(session.attach, frame.lease_id);
        }
        assert!(
            observed_other_slot,
            "live KWin stream did not publish another slot while the first slot was held"
        );

        release_now(session.attach, held_lease_id);

        let after_release_deadline = Instant::now() + Duration::from_secs(5);
        let mut observed_released_slot = false;
        while Instant::now() < after_release_deadline {
            let mut ready_cursor = 0;
            let wait_status = unsafe { ft_native_wait_frame(session.attach, last_cursor, 500_000_000, &mut ready_cursor) };
            if wait_status == FT_STATUS_TIMEOUT {
                continue;
            }
            assert_eq!(wait_status, FT_STATUS_OK, "unexpected wait status {wait_status}");

            let mut frame = empty_frame();
            let acquire_status = unsafe { ft_native_acquire_latest(session.attach, last_cursor, &mut frame) };
            if acquire_status == FT_STATUS_EMPTY {
                continue;
            }
            assert_eq!(acquire_status, FT_STATUS_OK, "unexpected acquire status {acquire_status}");
            last_cursor = frame.cursor;
            if frame.slot_id == held_slot_id {
                observed_released_slot = true;
            }
            release_now(session.attach, frame.lease_id);
            if observed_released_slot {
                break;
            }
        }
        assert!(
            observed_released_slot,
            "released PipeWire slot {held_slot_id} did not become reusable after the native lease released"
        );
    }
}
