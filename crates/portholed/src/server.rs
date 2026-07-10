#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};
#[cfg(target_os = "linux")]
use porthole_adapter_kwin::KWinAdapter;
use porthole_core::adapter::Adapter;
use porthole_transport::Endpoint;
use tracing::info;

#[cfg(target_os = "linux")]
use crate::kwin_bridge::{KWinBridge, spawn_session_service};
use crate::{
    agent_store::AgentPolicyStore,
    events::EventBus,
    routes::{
        agent_permissions as agent_permissions_route, attach as attach_route, attention as attention_route,
        capture_sessions as capture_sessions_route, close_focus as close_focus_route, content_rect as content_rect_route,
        events as events_route, info as info_route, input as input_route, launches as launches_route, place as place_route,
        pointer as pointer_route, replace as replace_route, screenshot as screenshot_route, system_permissions as system_permissions_route,
        wait as wait_route,
    },
    state::AppState,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/info", get(info_route::get_info))
        .route("/attention", get(attention_route::get_attention))
        .route("/displays", get(attention_route::get_displays))
        .route(
            "/agent-identities",
            post(agent_permissions_route::post_identity).get(agent_permissions_route::get_identities),
        )
        .route("/agent-identities/{agent_id}", get(agent_permissions_route::get_identity))
        .route(
            "/agent-identities/{agent_id}/revoke",
            post(agent_permissions_route::post_revoke_identity),
        )
        .route(
            "/agent-identities/{agent_id}/tokens",
            post(agent_permissions_route::post_identity_token),
        )
        .route(
            "/agent-identities/{agent_id}/tokens/{token_id}/revoke",
            post(agent_permissions_route::post_revoke_identity_token),
        )
        .route("/agent-permissions/requests", get(agent_permissions_route::get_requests))
        .route(
            "/agent-permissions/requests/{request_id}",
            get(agent_permissions_route::get_request),
        )
        .route(
            "/agent-permissions/requests/{request_id}/approve",
            post(agent_permissions_route::post_approve_request),
        )
        .route(
            "/agent-permissions/requests/{request_id}/deny",
            post(agent_permissions_route::post_deny_request),
        )
        .route("/agent-permissions/grants", get(agent_permissions_route::get_grants))
        .route(
            "/agent-permissions/grants/{grant_id}/revoke",
            post(agent_permissions_route::post_revoke_grant),
        )
        .route("/events", get(events_route::get_events))
        .route("/launches", post(launches_route::post_launches))
        .route("/surfaces/search", post(attach_route::post_search))
        .route("/surfaces/track", post(attach_route::post_track))
        .route("/surfaces/{id}/screenshot", post(screenshot_route::post_screenshot))
        .route("/surfaces/{id}/key", post(input_route::post_key))
        .route("/surfaces/{id}/text", post(input_route::post_text))
        .route("/surfaces/{id}/click", post(input_route::post_click))
        .route("/surfaces/{id}/scroll", post(input_route::post_scroll))
        .route("/surfaces/{id}/wait", post(wait_route::post_wait))
        .route("/surfaces/{id}/place", post(place_route::post_place))
        .route("/surfaces/{id}/replace", post(replace_route::post_replace))
        .route("/surfaces/{id}/close", post(close_focus_route::post_close))
        .route("/surfaces/{id}/focus", post(close_focus_route::post_focus))
        .route("/surfaces/{id}/content-rect", get(content_rect_route::get_content_rect))
        .route("/surfaces/{id}/pointer/move", post(pointer_route::post_pointer_move))
        .route("/system-permissions/request", post(system_permissions_route::post_request))
        .route("/capture-sessions/synthetic", post(capture_sessions_route::post_synthetic))
        .route("/capture-sessions/surfaces/{id}", post(capture_sessions_route::post_surface))
        .route(
            "/capture-sessions/{id}",
            get(capture_sessions_route::get_session).delete(capture_sessions_route::delete_session),
        )
        .with_state(state)
}

pub trait IntoControlEndpoint {
    fn into_control_endpoint(self) -> Endpoint;
}

#[cfg(unix)]
impl IntoControlEndpoint for PathBuf {
    fn into_control_endpoint(self) -> Endpoint {
        Endpoint::from_socket_path(self)
    }
}

impl IntoControlEndpoint for Endpoint {
    fn into_control_endpoint(self) -> Endpoint {
        self
    }
}

pub async fn serve(adapter: Arc<dyn Adapter>, endpoint: impl IntoControlEndpoint) -> std::io::Result<()> {
    let agent_store = AgentPolicyStore::open_default()
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    serve_with_agent_policy(adapter, endpoint, agent_store, EventBus::new()).await
}

pub async fn serve_with_agent_policy(
    adapter: Arc<dyn Adapter>,
    endpoint: impl IntoControlEndpoint,
    agent_store: AgentPolicyStore,
    events: EventBus,
) -> std::io::Result<()> {
    serve_with_agent_policy_inner(
        adapter,
        endpoint.into_control_endpoint(),
        agent_store,
        events,
        #[cfg(target_os = "linux")]
        None,
    )
    .await
}

#[cfg(target_os = "linux")]
pub async fn serve_with_agent_policy_and_kwin_bridge(
    kwin_adapter: Arc<KWinAdapter>,
    endpoint: impl IntoControlEndpoint,
    agent_store: AgentPolicyStore,
    events: EventBus,
    kwin_bridge: KWinBridge,
) -> std::io::Result<()> {
    let _kwin_bridge = spawn_session_service(kwin_bridge);
    let adapter: Arc<dyn Adapter> = kwin_adapter.clone();
    serve_with_agent_policy_inner(adapter, endpoint.into_control_endpoint(), agent_store, events, Some(kwin_adapter)).await
}

async fn serve_with_agent_policy_inner(
    adapter: Arc<dyn Adapter>,
    endpoint: Endpoint,
    agent_store: AgentPolicyStore,
    events: EventBus,
    #[cfg(target_os = "linux")] kwin_adapter: Option<Arc<KWinAdapter>>,
) -> std::io::Result<()> {
    info!(endpoint = %endpoint.display_name(), "portholed listening");
    #[cfg(unix)]
    let capture = {
        let capture_socket_path = endpoint.as_socket_path().with_file_name("capture-transfer.sock");
        crate::capture_registry::CaptureRegistry::with_fd_socket_and_agent_policy(capture_socket_path, agent_store.clone())?
    };
    #[cfg(windows)]
    let capture = crate::capture_registry::CaptureRegistry::disabled_with_agent_policy(agent_store.clone());
    let state = AppState::new_with_agent_policy_and_capture(adapter, capture, agent_store, events);
    #[cfg(target_os = "linux")]
    let state = if let Some(kwin_adapter) = kwin_adapter {
        state.with_kwin_adapter(kwin_adapter)
    } else {
        state
    };
    let app = build_router(state);
    porthole_transport::serve(endpoint, app).await
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        collections::BTreeMap,
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        sync::Arc,
    };

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use capture_transfer::control_page::VideoTrackControlPage;
    use porthole_core::{
        agent_policy::{ActionClass, DurationSpec, TargetSelector},
        in_memory::InMemoryAdapter,
        surface::{PlatformSurfaceRef, SurfaceInfo},
    };
    use porthole_protocol::capture_sessions::{CreateCaptureSessionResponse, LatestVideoFrameResponse};
    use tower::ServiceExt;

    use super::*;

    async fn router_with_tracked_surface() -> (Router, String, String) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
        let id = info.id.clone();
        state.handles.insert(info).await;
        let token = authorize_surface(
            &state,
            &id,
            vec![ActionClass::Drive, ActionClass::Manage, ActionClass::Observe, ActionClass::Record],
        )
        .await;
        (build_router(state), id.to_string(), token)
    }

    async fn router_with_authorized_tracked_surface() -> (Router, String, String) {
        router_with_tracked_surface().await
    }

    async fn authorize_surface(state: &AppState, id: &porthole_core::SurfaceId, actions: Vec<ActionClass>) -> String {
        let identity = state.agent_store.create_identity("agent", None, 1_000).await.unwrap();
        let request = state
            .agent_store
            .create_pending_request(
                identity.agent_id.clone(),
                TargetSelector::Surface { surface_id: id.clone() },
                actions,
                None,
                1_001,
            )
            .await
            .unwrap();
        state
            .agent_store
            .approve_request(&request.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), 1_002)
            .await
            .unwrap();
        identity.token
    }

    async fn authorize_target(state: &AppState, target: TargetSelector, actions: Vec<ActionClass>) -> String {
        let identity = state.agent_store.create_identity("agent", None, 1_000).await.unwrap();
        let request = state
            .agent_store
            .create_pending_request(identity.agent_id.clone(), target, actions, None, 1_001)
            .await
            .unwrap();
        state
            .agent_store
            .approve_request(&request.request_id, DurationSpec::Persistent, Vec::new(), 1_002)
            .await
            .unwrap();
        identity.token
    }

    async fn post(router: Router, uri: &str, body: serde_json::Value) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    async fn post_with_token(router: Router, uri: &str, token: &str, body: serde_json::Value) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    async fn get_with_token(router: Router, uri: &str, token: &str) -> axum::http::Response<Body> {
        let req = Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        router.oneshot(req).await.unwrap()
    }

    async fn router_with_capture_socket() -> (Router, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new_with_capture_socket(adapter, temp.path().join("capture-transfer.sock")).unwrap();
        (build_router(state), temp)
    }

    #[tokio::test]
    async fn synthetic_capture_session_serves_latest_frame_fd() {
        let (router, _temp) = router_with_capture_socket().await;
        let res = post(router.clone(), "/capture-sessions/synthetic", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let created: CreateCaptureSessionResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(created.source_id, 1);
        assert_eq!(created.track_id, 1);
        assert_eq!(created.status, "ready");
        assert_eq!(created.status_message, None);

        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("/capture-sessions/{}", created.session_id))
            .body(Body::empty())
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let session: porthole_protocol::capture_sessions::CaptureSessionResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(session.status, "ready");
        assert_eq!(session.status_message, None);
        assert_eq!(session.width, 2);
        assert_eq!(session.height, 1);

        let mut stream = UnixStream::connect(&created.fd_socket_path).unwrap();
        let reader_stream = stream.try_clone().unwrap();
        let mut reader = BufReader::with_capacity(1, reader_stream);
        let mut pools = BTreeMap::new();
        let frame = request_latest_frame_on_stream(&mut stream, &mut reader, &created, &mut pools);
        assert_eq!(frame.session_id, created.session_id);
        assert_eq!(frame.track_id, created.track_id);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.clock_domain, "unknown");
        assert_eq!(frame.color_space, "unknown");
        assert_eq!(frame.sync_kind, "cpu_copy_complete");
        assert_eq!(frame.damage_kind, "full_frame");
        assert_eq!(frame.damage_base_sequence, 1);
        assert_eq!(frame.dropped_before_publish, 0);
        assert_eq!(frame.producer_drop_count, 0);
        assert_eq!(frame.producer_cursor, 1);
        assert_eq!(frame.evicted_count, 0);
        assert_eq!(frame.consumer_skipped_count, 0);
        assert_ne!(frame.pool_id, 0);
        assert_eq!(frame.slot_id, 0);
        assert_eq!(frame.payload_len, frame.len);
        assert!(frame.payload_offset + frame.payload_len <= frame.payload_map_len);
        release_frame_on_stream(&mut stream, frame.lease_id);
    }

    #[tokio::test]
    async fn capture_fd_socket_serves_multiple_leased_frames_on_one_connection() {
        let (router, _temp) = router_with_capture_socket().await;
        let res = post(router, "/capture-sessions/synthetic", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let created: CreateCaptureSessionResponse = serde_json::from_slice(&body).unwrap();

        let mut stream = UnixStream::connect(&created.fd_socket_path).unwrap();
        let reader_stream = stream.try_clone().unwrap();
        let mut reader = BufReader::with_capacity(1, reader_stream);
        let mut pools = BTreeMap::new();

        let first = request_latest_frame_on_stream(&mut stream, &mut reader, &created, &mut pools);
        assert_ne!(first.lease_id, 0);
        release_frame_on_stream(&mut stream, first.lease_id);

        let second = request_latest_frame_on_stream(&mut stream, &mut reader, &created, &mut pools);
        assert_ne!(second.lease_id, 0);
        assert_ne!(second.lease_id, first.lease_id);
        release_frame_on_stream(&mut stream, second.lease_id);
    }

    #[tokio::test]
    async fn capture_fd_socket_registers_reusable_cpu_pool_once() {
        let (router, _temp) = router_with_capture_socket().await;
        let res = post(router, "/capture-sessions/synthetic", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let created: CreateCaptureSessionResponse = serde_json::from_slice(&body).unwrap();

        let mut stream = UnixStream::connect(&created.fd_socket_path).unwrap();
        let reader_stream = stream.try_clone().unwrap();
        let mut reader = BufReader::with_capacity(1, reader_stream);

        request_latest_frame(&mut stream, &created);
        let control = read_json_line(&mut reader);
        assert_eq!(control["op"], "register_video_control_page");
        assert_eq!(control["session_id"], created.session_id);
        assert_eq!(control["track_id"], created.track_id);
        assert_ne!(control["consumer_id"].as_u64().unwrap(), 0);
        assert_eq!(control["consumer_slot"], 0);
        let control_fd = capture_transfer::fdpass::recv_fd(&stream).unwrap();
        let control_page = VideoTrackControlPage::map_read_only(control_fd, control["map_len"].as_u64().unwrap() as usize).unwrap();
        assert_eq!(control_page.validate_header().unwrap().producer_cursor, 1);

        let pool = read_json_line(&mut reader);
        assert_eq!(pool["op"], "register_cpu_pool");
        assert_eq!(pool["session_id"], created.session_id);
        assert_eq!(pool["track_id"], created.track_id);
        assert_ne!(pool["pool_id"].as_u64().unwrap(), 0);
        assert_eq!(pool["slot_count"], 3);
        let _pool_fd = capture_transfer::fdpass::recv_fd(&stream).unwrap();

        let first = read_json_line(&mut reader);
        assert_eq!(first["op"], "video_frame");
        assert_eq!(first["producer_cursor"], 1);
        let entry = control_page.shadow_read_entry_for_cursor(1).unwrap();
        assert_eq!(entry.sequence, first["sequence"].as_u64().unwrap());
        assert_eq!(entry.pool_id, first["pool_id"].as_u64().unwrap());
        assert_eq!(entry.payload_len, first["payload_len"].as_u64().unwrap());
        let first_lease = first["lease_id"].as_u64().unwrap();
        assert_ne!(first_lease, 0);
        assert_eq!(first["pool_id"], pool["pool_id"]);
        release_frame_on_stream(&mut stream, first_lease);

        request_latest_frame(&mut stream, &created);
        let second = read_json_line(&mut reader);
        assert_eq!(second["op"], "video_frame");
        assert_eq!(second["producer_cursor"], first["producer_cursor"]);
        let second_lease = second["lease_id"].as_u64().unwrap();
        assert_ne!(second_lease, 0);
        assert_eq!(second["pool_id"], pool["pool_id"]);
        release_frame_on_stream(&mut stream, second_lease);
    }

    fn release_frame_on_stream(stream: &mut UnixStream, lease_id: u64) {
        writeln!(
            stream,
            "{}",
            serde_json::json!({
                "op": "release_video_frame",
                "lease_id": lease_id
            })
        )
        .unwrap();
    }

    fn request_latest_frame(stream: &mut UnixStream, created: &CreateCaptureSessionResponse) {
        writeln!(
            stream,
            "{}",
            serde_json::json!({
                "op": "latest_video_frame",
                "session_id": created.session_id,
                "track_id": created.track_id
            })
        )
        .unwrap();
    }

    fn authorize_capture_transfer_stream(stream: &mut UnixStream, created: &CreateCaptureSessionResponse, token: &str) {
        writeln!(
            stream,
            "{}",
            serde_json::json!({
                "op": "authorize",
                "session_id": created.session_id,
                "bearer_token": token
            })
        )
        .unwrap();
    }

    fn read_json_line(reader: &mut BufReader<UnixStream>) -> serde_json::Value {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(line.trim_end()).unwrap()
    }

    fn request_latest_frame_on_stream(
        stream: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        created: &CreateCaptureSessionResponse,
        pools: &mut BTreeMap<(u64, u64), capture_transfer::shm::SharedMemorySegment>,
    ) -> LatestVideoFrameResponse {
        request_latest_frame(stream, created);

        let frame = loop {
            let value = read_json_line(reader);
            match value["op"].as_str() {
                Some("register_video_control_page") => {
                    assert_ne!(value["consumer_id"].as_u64().unwrap(), 0);
                    let fd = capture_transfer::fdpass::recv_fd(stream).unwrap();
                    let page = VideoTrackControlPage::map_read_only(fd, value["map_len"].as_u64().unwrap() as usize).unwrap();
                    page.validate_header().unwrap();
                }
                Some("register_cpu_pool") => {
                    let fd = capture_transfer::fdpass::recv_fd(stream).unwrap();
                    let key = (value["track_id"].as_u64().unwrap(), value["pool_id"].as_u64().unwrap());
                    // Pool fds are anonymous shm objects: mmap-only, no read().
                    let map_len = value["payload_map_len"].as_u64().unwrap() as usize;
                    pools.insert(key, capture_transfer::shm::SharedMemorySegment::map_read_only(fd, map_len).unwrap());
                }
                Some("video_frame") => break serde_json::from_value::<LatestVideoFrameResponse>(value).unwrap(),
                other => panic!("unexpected capture fd socket response {other:?}"),
            }
        };
        assert_eq!(frame.session_id, created.session_id);
        assert_eq!(frame.track_id, created.track_id);
        assert_eq!(frame.payload_len, frame.len);

        let key = (frame.track_id, frame.pool_id);
        let mapping = pools.get(&key).expect("frame references registered pool");
        let bytes = mapping.slice_at(frame.payload_offset as usize, frame.payload_len as usize);
        assert_eq!(bytes, &[0, 64, 128, 255, 255, 64, 128, 255]);
        frame
    }

    #[tokio::test]
    async fn delete_capture_session_removes_it() {
        let (router, _temp) = router_with_capture_socket().await;
        let res = post(router.clone(), "/capture-sessions/synthetic", serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let created: CreateCaptureSessionResponse = serde_json::from_slice(&body).unwrap();

        let req = Request::builder()
            .method(Method::DELETE)
            .uri(format!("/capture-sessions/{}", created.session_id))
            .body(Body::empty())
            .unwrap();
        let res = router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);

        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("/capture-sessions/{}", created.session_id))
            .body(Body::empty())
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn surface_capture_session_serves_latest_frame_fd() {
        let temp = tempfile::tempdir().unwrap();
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new_with_capture_socket(adapter, temp.path().join("capture-transfer.sock")).unwrap();
        let info = SurfaceInfo::window(porthole_core::SurfaceId::from("surf_test"), 1);
        let surface_id = info.id.clone();
        state.handles.insert(info).await;
        let token = authorize_surface(&state, &surface_id, vec![ActionClass::Observe, ActionClass::Record]).await;
        let router = build_router(state);

        let res = post_with_token(
            router.clone(),
            "/capture-sessions/surfaces/surf_test",
            &token,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let created: CreateCaptureSessionResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(created.source_id, 1);
        assert_eq!(created.track_id, 1);
        assert_eq!(created.status, "ready");
        assert_eq!(created.status_message, None);

        let mut stream = UnixStream::connect(&created.fd_socket_path).unwrap();
        let reader_stream = stream.try_clone().unwrap();
        let mut reader = BufReader::with_capacity(1, reader_stream);
        let mut pools = BTreeMap::new();
        authorize_capture_transfer_stream(&mut stream, &created, &token);
        let frame = request_latest_frame_on_stream(&mut stream, &mut reader, &created, &mut pools);
        assert_eq!(frame.session_id, created.session_id);
        assert_eq!(frame.track_id, created.track_id);
        assert_eq!(frame.timestamp_ns, 123_456_789);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 1);
        assert_eq!(frame.clock_domain, "unix_time");
        assert_eq!(frame.color_space, "unknown");
        assert_eq!(frame.sync_kind, "cpu_copy_complete");
        assert_eq!(frame.damage_kind, "full_frame");
        assert_eq!(frame.damage_base_sequence, 1);
        assert_ne!(frame.pool_id, 0);
        assert_eq!(frame.slot_id, 0);
        assert_eq!(frame.payload_len, frame.len);
        assert!(frame.payload_offset + frame.payload_len <= frame.payload_map_len);
        release_frame_on_stream(&mut stream, frame.lease_id);
    }

    #[tokio::test]
    async fn get_info_returns_adapter_info() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(info.adapters.len(), 1);
        assert_eq!(info.adapters[0].name, "in-memory");
        assert_eq!(info.surface_count, 0);
    }

    #[tokio::test]
    async fn get_info_reports_alive_surface_count() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let alive = porthole_core::SurfaceInfo::window(porthole_core::SurfaceId::from("surf_alive"), 1);
        let dead = porthole_core::SurfaceInfo::window(porthole_core::SurfaceId::from("surf_dead"), 2);
        let dead_id = dead.id.clone();
        state.handles.insert(alive).await;
        state.handles.insert(dead).await;
        state.handles.mark_dead(&dead_id).await.unwrap();
        let router = build_router(state);

        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(info.surface_count, 1);
    }

    #[tokio::test]
    async fn post_launch_then_screenshot_roundtrips() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let launch_token = authorize_target(&state, TargetSelector::LaunchedByAgent, vec![ActionClass::Manage]).await;
        let router = build_router(state.clone());
        let launch_body = serde_json::json!({
            "kind": { "type": "process", "app": "test", "args": [] },
            "require_confidence": "strong"
        });
        let res = post_with_token(router.clone(), "/launches", &launch_token, launch_body).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let launch: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        let screenshot_token = authorize_surface(
            &state,
            &porthole_core::SurfaceId::from(launch.surface_id.as_str()),
            vec![ActionClass::Observe],
        )
        .await;

        let res = post_with_token(
            router,
            &format!("/surfaces/{}/screenshot", launch.surface_id),
            &screenshot_token,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 4 * 1024 * 1024).await.unwrap();
        let shot: porthole_protocol::screenshot::ScreenshotResponse = serde_json::from_slice(&body).unwrap();
        assert!(!shot.png_base64.is_empty());
    }

    #[tokio::test]
    async fn post_key_sends_events() {
        let (router, id, token) = router_with_authorized_tracked_surface().await;
        let res = post_with_token(
            router,
            &format!("/surfaces/{id}/key"),
            &token,
            serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::KeyResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.events_sent, 1);
    }

    #[tokio::test]
    async fn post_key_with_unsupported_name_returns_bad_request() {
        let (router, id, token) = router_with_authorized_tracked_surface().await;
        let res = post_with_token(
            router,
            &format!("/surfaces/{id}/key"),
            &token,
            serde_json::json!({ "events": [{ "key": "NotAKey" }] }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_text_reports_char_count() {
        let (router, id, token) = router_with_authorized_tracked_surface().await;
        let res = post_with_token(router, &format!("/surfaces/{id}/text"), &token, serde_json::json!({ "text": "hi" })).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::input::TextResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.chars_sent, 2);
    }

    #[tokio::test]
    async fn post_close_marks_surface_dead() {
        let (router, id, token) = router_with_tracked_surface().await;
        let res = post_with_token(router.clone(), &format!("/surfaces/{id}/close"), &token, serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        // Subsequent operations should 410 (GONE)
        let res = post_with_token(router, &format!("/surfaces/{id}/focus"), &token, serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::GONE);
    }

    #[tokio::test]
    async fn get_attention_returns_default() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = get_with_token(router, "/attention", &token).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_displays_returns_list() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Observe]).await;
        let router = build_router(state);
        let res = get_with_token(router, "/displays", &token).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::attention::DisplaysResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.displays.len(), 1);
    }

    #[tokio::test]
    async fn post_search_returns_candidates_from_adapter_script() {
        use porthole_core::search::Candidate;
        let adapter = Arc::new(InMemoryAdapter::new());
        let candidate = Candidate {
            ref_: "ref_abc".into(),
            app_name: Some("X".into()),
            title: Some("t".into()),
            pid: 1,
            platform_ref: PlatformSurfaceRef::macos(7),
        };
        adapter.set_next_search_result(Ok(vec![candidate])).await;
        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(router, "/surfaces/search", &token, serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::search::SearchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.candidates.len(), 1);
    }

    #[tokio::test]
    async fn post_search_with_empty_adapter_result_returns_empty_candidates_list() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter.set_next_search_result(Ok(vec![])).await;
        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(router, "/surfaces/search", &token, serde_json::json!({})).await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::search::SearchResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.candidates.is_empty());
    }

    #[tokio::test]
    async fn post_track_mints_handle_and_idempotent_reuse() {
        use porthole_core::{
            search::encode_ref,
            surface::{SurfaceId, SurfaceInfo},
        };

        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Manage]).await;
        let router = build_router(state);

        let r = encode_ref(1, PlatformSurfaceRef::macos(7));
        let body = serde_json::json!({ "ref": r });

        // First call: script surface_alive to return an alive surface.
        let mut info = SurfaceInfo::window(SurfaceId::new(), 1);
        info.platform_ref = Some(PlatformSurfaceRef::macos(7));
        info.app_name = Some("X".into());
        adapter.set_next_surface_alive_result(Ok(Some(info))).await;
        let res = post_with_token(router.clone(), "/surfaces/track", &token, body.clone()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let first_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let first: porthole_protocol::search::TrackResponse = serde_json::from_slice(&first_body).unwrap();
        assert!(!first.reused_existing_handle);

        // Second call: script another alive surface with the same platform ref.
        // track_or_get should find the existing handle and return reused=true.
        let mut info2 = SurfaceInfo::window(SurfaceId::new(), 1);
        info2.platform_ref = Some(PlatformSurfaceRef::macos(7));
        info2.app_name = Some("X".into());
        adapter.set_next_surface_alive_result(Ok(Some(info2))).await;
        let res = post_with_token(router, "/surfaces/track", &token, body).await;
        assert_eq!(res.status(), StatusCode::OK);
        let second_body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let second: porthole_protocol::search::TrackResponse = serde_json::from_slice(&second_body).unwrap();
        assert!(second.reused_existing_handle);
        assert_eq!(second.surface_id, first.surface_id);
    }

    #[tokio::test]
    async fn post_track_with_malformed_ref_returns_not_found() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::AllSurfaces, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(router, "/surfaces/track", &token, serde_json::json!({ "ref": "junk" })).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_replace_inherits_snapshot_when_no_placement() {
        use porthole_core::{
            display::{DisplayId, Rect},
            placement::GeometrySnapshot,
            surface::{SurfaceId, SurfaceInfo},
        };

        let adapter = Arc::new(InMemoryAdapter::new());
        // Seed an alive handle with a platform ref.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(50));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect {
                    x: 10.0,
                    y: 20.0,
                    w: 500.0,
                    h: 400.0,
                },
            }))
            .await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::Applied);
    }

    #[tokio::test]
    async fn post_replace_with_empty_placement_does_not_inherit() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(51));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {}
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::NotRequested);
    }

    #[tokio::test]
    async fn post_replace_force_place_applies_placement_to_preexisting_surface() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(51));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        let mut outcome = InMemoryAdapter::make_default_launch_outcome(100);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {
                    "on_display": "primary",
                    "geometry": { "x": 0.0, "y": 0.0, "w": 500.0, "h": 500.0 }
                },
                "force_place": true
            }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::Applied);
        assert_eq!(adapter.place_surface_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn post_launches_rejects_url_artifact() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let res = post(
            router,
            "/launches",
            serde_json::json!({
                "kind": { "type": "artifact", "path": "https://example.com" }
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn info_lists_slice_c_capabilities() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let router = build_router(AppState::new(adapter));
        let req = Request::builder().method(Method::GET).uri("/info").body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let info: porthole_protocol::info::InfoResponse = serde_json::from_slice(&body).unwrap();
        let caps = &info.adapters[0].capabilities;
        for expected in &["launch_artifact", "placement", "replace", "auto_dismiss"] {
            assert!(caps.contains(&expected.to_string()), "missing capability: {expected}");
        }
    }

    #[tokio::test]
    async fn post_replace_close_failure_returns_409_with_old_handle_alive_body() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        // Seed an alive handle.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(50));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        // Script close to fail.
        adapter
            .set_next_close_result(Err(porthole_core::PortholeError::new(
                porthole_core::ErrorCode::CloseFailed,
                "save dialog blocking",
            )))
            .await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({ "kind": { "type": "artifact", "path": "/tmp/x.pdf" } }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        // Typed error code is preserved — still close_failed.
        assert_eq!(err.code, porthole_core::ErrorCode::CloseFailed);
        let details = err.details.expect("details populated");
        assert_eq!(details.get("old_handle_alive").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn post_replace_preserves_system_permission_needed_from_close() {
        use porthole_core::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(50));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        adapter
            .set_next_close_result(Err(porthole_core::PortholeError::new(
                porthole_core::ErrorCode::SystemPermissionNeeded,
                "AX denied",
            )))
            .await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({ "kind": { "type": "artifact", "path": "/tmp/x.pdf" } }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::SystemPermissionNeeded);
        let details = err.details.expect("details populated");
        assert_eq!(
            details.get("old_handle_alive").and_then(|v| v.as_bool()),
            Some(true),
            "system_permission_needed on close means surface is likely still alive"
        );
    }

    #[tokio::test]
    async fn post_replace_launch_failure_after_close_returns_old_handle_alive_false() {
        use porthole_core::{
            in_memory::InMemoryAdapter,
            surface::{SurfaceId, SurfaceInfo},
        };

        let adapter = Arc::new(InMemoryAdapter::new());
        // Alive handle to replace.
        let mut old = SurfaceInfo::window(SurfaceId::new(), 1);
        old.platform_ref = Some(PlatformSurfaceRef::macos(50));
        let old_id = old.id.clone();
        let state = AppState::new(adapter.clone());
        state.handles.insert(old).await;
        let token = authorize_surface(&state, &old_id, vec![ActionClass::Manage]).await;

        // Close will succeed (default). Make the artifact launch return weak
        // confidence, which will fail the Strong requirement.
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(999);
        outcome.confidence = porthole_core::adapter::Confidence::Weak;
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let router = build_router(state);
        let res = post_with_token(
            router,
            &format!("/surfaces/{old_id}/replace"),
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_confidence": "strong"
            }),
        )
        .await;
        // Expect non-success: LaunchCorrelationAmbiguous → 409 CONFLICT
        assert!(!res.status().is_success());
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        // Regardless of exact code, details should include old_handle_alive: false
        let details = err.details.expect("details populated after post-close failure");
        assert_eq!(
            details.get("old_handle_alive").and_then(|v| v.as_bool()),
            Some(false),
            "post-close failure must report old_handle_alive: false"
        );
    }

    #[tokio::test]
    async fn post_launches_require_fresh_returns_409_with_ref_in_body() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(100);
        outcome.surface_was_preexisting = true;
        outcome.surface.platform_ref = Some(PlatformSurfaceRef::macos(321));
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let state = AppState::new(adapter);
        let token = authorize_target(&state, TargetSelector::LaunchedByAgent, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(
            router,
            "/launches",
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_fresh_surface": true
            }),
        )
        .await;
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::LaunchReturnedExisting);
        let details = err.details.expect("details populated");
        assert!(details.get("ref").is_some());
        assert_eq!(details["platform_ref"]["platform"].as_str(), Some("macos"));
        assert_eq!(details["platform_ref"]["cg_window_id"].as_u64(), Some(321));
    }

    #[tokio::test]
    async fn post_launches_force_place_applies_placement_to_preexisting_surface() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(100);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let state = AppState::new(adapter.clone());
        let token = authorize_target(&state, TargetSelector::LaunchedByAgent, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(
            router,
            "/launches",
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "placement": {
                    "on_display": "primary",
                    "geometry": { "x": 0.0, "y": 0.0, "w": 500.0, "h": 500.0 }
                },
                "force_place": true
            }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::OK);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let resp: porthole_protocol::launches::LaunchResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.placement, porthole_core::placement::PlacementOutcome::Applied);
        assert_eq!(adapter.place_surface_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn post_launches_rejects_require_fresh_with_force_place_before_launch() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let state = AppState::new(adapter.clone());
        let token = authorize_target(&state, TargetSelector::LaunchedByAgent, vec![ActionClass::Manage]).await;
        let router = build_router(state);
        let res = post_with_token(
            router,
            "/launches",
            &token,
            serde_json::json!({
                "kind": { "type": "artifact", "path": "/tmp/x.pdf" },
                "require_fresh_surface": true,
                "force_place": true
            }),
        )
        .await;

        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let err: porthole_protocol::error::WireError = serde_json::from_slice(&body).unwrap();
        assert_eq!(err.code, porthole_core::ErrorCode::InvalidArgument);
        assert!(adapter.launch_artifact_calls().await.is_empty());
    }
}
