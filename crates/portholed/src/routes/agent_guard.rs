use axum::http::HeaderMap;
use porthole_core::{
    ErrorCode, SurfaceId,
    agent_policy::{
        ActionClass, AgentContext, AgentId, AuthorizationDecision, GrantId, PermissionRequestId, TargetContext, TargetSelector,
    },
};
use porthole_protocol::{
    agent_permissions::{AgentPermissionDuration, AgentPermissionNeededDetails, AgentPermissionTarget},
    error::WireError,
};

use crate::{agent_store::ExecutionAuditRecord, events::AgentEvent, routes::errors::ApiError, state::AppState};

#[derive(Clone, Debug)]
pub struct AuthorizedRouteExecution {
    pub agent_id: AgentId,
    pub grant_id: GrantId,
    pub origin_request_id: Option<PermissionRequestId>,
    pub consumes_grant: bool,
    pub target: TargetSelector,
    pub actions: Vec<ActionClass>,
}

struct PermissionNeededContext {
    target: TargetSelector,
    surface_id: Option<SurfaceId>,
    actions: Vec<ActionClass>,
    reason: Option<String>,
    recommended_duration: AgentPermissionDuration,
    publish_new_request: bool,
}

pub async fn authorize_surface_actions(
    state: &AppState,
    headers: &HeaderMap,
    surface_id: &str,
    actions: &[ActionClass],
    reason: Option<&str>,
) -> Result<AuthorizedRouteExecution, ApiError> {
    authorize_surface_actions_inner(state, headers, surface_id, actions, reason, false).await
}

pub async fn authorize_launch_actions(
    state: &AppState,
    headers: &HeaderMap,
    actions: &[ActionClass],
    reason: Option<&str>,
) -> Result<AuthorizedRouteExecution, ApiError> {
    let agent_id = authenticated_agent_id(state, headers).await?;
    let target = TargetSelector::LaunchedByAgent;
    let target_context = TargetContext {
        surface_id: None,
        app_bundle_id: None,
        executable_path: None,
        app_name: None,
        launched_by_agent: Some(agent_id.clone()),
        frontmost_surface_id: None,
        surface_alive: true,
    };
    authorize_target_actions(
        state,
        agent_id,
        target,
        target_context,
        None,
        actions,
        reason,
        AgentPermissionDuration::Once,
        false,
    )
    .await
}

pub async fn authorize_all_surfaces_actions(
    state: &AppState,
    headers: &HeaderMap,
    actions: &[ActionClass],
    reason: Option<&str>,
) -> Result<AuthorizedRouteExecution, ApiError> {
    let agent_id = authenticated_agent_id(state, headers).await?;
    let target = TargetSelector::AllSurfaces;
    let target_context = TargetContext {
        surface_id: None,
        app_bundle_id: None,
        executable_path: None,
        app_name: None,
        launched_by_agent: None,
        frontmost_surface_id: None,
        surface_alive: true,
    };
    authorize_target_actions(
        state,
        agent_id,
        target,
        target_context,
        None,
        actions,
        reason,
        AgentPermissionDuration::Once,
        false,
    )
    .await
}

async fn authorize_surface_actions_inner(
    state: &AppState,
    headers: &HeaderMap,
    surface_id: &str,
    actions: &[ActionClass],
    reason: Option<&str>,
    retried_once_consumption: bool,
) -> Result<AuthorizedRouteExecution, ApiError> {
    let agent_id = authenticated_agent_id(state, headers).await?;
    let surface_id = SurfaceId::from(surface_id);
    let surface = state.handles.require_alive(&surface_id).await?;
    let target = TargetSelector::Surface {
        surface_id: surface_id.clone(),
    };
    let target_context = TargetContext {
        surface_id: Some(surface_id.clone()),
        app_bundle_id: None,
        executable_path: None,
        app_name: surface.app_name,
        launched_by_agent: None,
        frontmost_surface_id: None,
        surface_alive: true,
    };
    authorize_target_actions(
        state,
        agent_id,
        target,
        target_context,
        Some(surface_id),
        actions,
        reason,
        AgentPermissionDuration::UntilSurfaceGone,
        retried_once_consumption,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn authorize_target_actions(
    state: &AppState,
    agent_id: AgentId,
    target: TargetSelector,
    target_context: TargetContext,
    details_surface_id: Option<SurfaceId>,
    actions: &[ActionClass],
    reason: Option<&str>,
    recommended_duration: AgentPermissionDuration,
    retried_once_consumption: bool,
) -> Result<AuthorizedRouteExecution, ApiError> {
    let snapshot = state.agent_store.load_policy_snapshot(&agent_id).await?;
    let agent = AgentContext {
        agent_id: agent_id.clone(),
    };

    match snapshot.authorize(&agent, &target_context, actions, now_unix_ms()) {
        AuthorizationDecision::Allowed { grant_id, consumes_grant } => {
            let grant = state.agent_store.get_grant(&grant_id).await?.ok_or_else(|| {
                ApiError(WireError {
                    code: ErrorCode::InternalError,
                    message: "authorized grant was not found".into(),
                    details: None,
                })
            })?;
            if consumes_grant && !state.agent_store.try_consume_once_grant(&grant_id, now_unix_ms()).await? {
                if retried_once_consumption {
                    return permission_needed(
                        state,
                        agent_id,
                        PermissionNeededContext {
                            target,
                            surface_id: details_surface_id,
                            actions: actions.to_vec(),
                            reason: reason.map(ToOwned::to_owned),
                            recommended_duration,
                            publish_new_request: false,
                        },
                    )
                    .await;
                }
                return Box::pin(authorize_target_actions(
                    state,
                    agent_id,
                    target,
                    target_context,
                    details_surface_id,
                    actions,
                    reason,
                    recommended_duration,
                    true,
                ))
                .await;
            }
            Ok(AuthorizedRouteExecution {
                agent_id,
                grant_id,
                origin_request_id: grant.origin_request_id,
                consumes_grant,
                target,
                actions: actions.to_vec(),
            })
        }
        AuthorizationDecision::Denied { .. } => Err(ApiError(WireError {
            code: ErrorCode::AgentPermissionDenied,
            message: "agent permission denied".into(),
            details: None,
        })),
        AuthorizationDecision::NeedsPermission => {
            permission_needed(
                state,
                agent_id,
                PermissionNeededContext {
                    target,
                    surface_id: details_surface_id,
                    actions: actions.to_vec(),
                    reason: reason.map(ToOwned::to_owned),
                    recommended_duration,
                    publish_new_request: true,
                },
            )
            .await
        }
    }
}

pub async fn complete_route_execution(state: &AppState, execution: AuthorizedRouteExecution, route: &'static str) -> Result<(), ApiError> {
    let result = state
        .agent_store
        .insert_execution_audit(ExecutionAuditRecord {
            agent_id: execution.agent_id,
            request_id: execution.origin_request_id,
            route: route.into(),
            target: execution.target,
            actions: execution.actions,
            grant_id: Some(execution.grant_id),
            denial_id: None,
            executed_at_unix_ms: now_unix_ms(),
        })
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, "failed to write agent route execution audit after route action");
    }
    Ok(())
}

async fn permission_needed(
    state: &AppState,
    agent_id: AgentId,
    context: PermissionNeededContext,
) -> Result<AuthorizedRouteExecution, ApiError> {
    let PermissionNeededContext {
        target,
        surface_id,
        actions,
        reason,
        recommended_duration,
        publish_new_request,
    } = context;
    let wire_target: AgentPermissionTarget = target.clone().into();
    let (request, created) = state
        .agent_store
        .find_or_create_pending_request(agent_id.clone(), target, actions.clone(), reason, now_unix_ms())
        .await?;
    if publish_new_request && created {
        state.events.publish(AgentEvent::AgentPermissionRequested {
            request_id: request.request_id.clone(),
            agent_id: agent_id.clone(),
        });
    }
    Err(ApiError(WireError {
        code: ErrorCode::AgentPermissionNeeded,
        message: "agent permission required".into(),
        details: serde_json::to_value(AgentPermissionNeededDetails {
            request_id: request.request_id,
            agent_id,
            target: wire_target,
            surface_id,
            actions,
            recommended_duration,
        })
        .ok(),
    }))
}

async fn authenticated_agent_id(state: &AppState, headers: &HeaderMap) -> Result<AgentId, ApiError> {
    let token = bearer_token(headers)?;
    state
        .agent_store
        .authenticate_agent_token(token)
        .await?
        .ok_or_else(identity_required)
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(identity_required());
    };
    let Ok(value) = value.to_str() else {
        return Err(identity_required());
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return Err(identity_required());
    };
    if scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() {
        Ok(token)
    } else {
        Err(identity_required())
    }
}

fn identity_required() -> ApiError {
    ApiError(WireError {
        code: ErrorCode::AgentIdentityRequired,
        message: "agent identity bearer token required".into(),
        details: None,
    })
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use porthole_core::{
        ErrorCode, SurfaceId,
        agent_policy::{ActionClass, DurationSpec},
        in_memory::InMemoryAdapter,
        surface::SurfaceInfo,
    };
    use porthole_protocol::{
        agent_permissions::{AgentPermissionNeededDetails, AgentPermissionTarget},
        capture_sessions::CreateCaptureSessionResponse,
        error::WireError,
        input::TextResponse,
    };
    use tower::ServiceExt;

    use crate::{
        agent_store::AgentPolicyStore,
        capture_registry::CaptureRegistry,
        events::{AgentEvent, EventBus},
        server::build_router,
        state::AppState,
    };

    const NOW: u64 = 1_000;

    struct TestHarness {
        router: axum::Router,
        surface_id: SurfaceId,
        store: AgentPolicyStore,
        token: String,
        events: tokio::sync::broadcast::Receiver<AgentEvent>,
        adapter: Arc<InMemoryAdapter>,
        _temp: tempfile::TempDir,
    }

    async fn harness() -> TestHarness {
        let temp = tempfile::tempdir().unwrap();
        let adapter = Arc::new(InMemoryAdapter::new());
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let events = EventBus::new();
        let event_rx = events.subscribe();
        let capture = CaptureRegistry::with_fd_socket(temp.path().join("capture-transfer.sock")).unwrap();
        let state = AppState::new_with_agent_policy_and_capture(adapter.clone(), capture, store.clone(), events);
        let info = SurfaceInfo::window(SurfaceId::new(), 4242);
        let surface_id = info.id.clone();
        state.handles.insert(info).await;
        let identity = store.create_identity("agent", None, NOW).await.unwrap();
        let router = build_router(state);
        TestHarness {
            router,
            surface_id,
            store,
            token: identity.token,
            events: event_rx,
            adapter,
            _temp: temp,
        }
    }

    async fn post_text(router: axum::Router, surface_id: &SurfaceId, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        let authorization = token.map(|token| format!("Bearer {token}"));
        post_text_with_auth_header(router, surface_id, authorization.as_deref()).await
    }

    async fn post_json(
        router: axum::Router,
        uri: impl Into<String>,
        token: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(uri.into())
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let req = builder.body(Body::from(body.to_string())).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({}));
        (status, json)
    }

    async fn get_json(router: axum::Router, uri: impl Into<String>, token: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(Method::GET).uri(uri.into());
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let req = builder.body(Body::empty()).unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({}));
        (status, json)
    }

    async fn post_text_with_auth_header(
        router: axum::Router,
        surface_id: &SurfaceId,
        authorization: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(format!("/surfaces/{surface_id}/text"))
            .header("content-type", "application/json");
        if let Some(authorization) = authorization {
            builder = builder.header("authorization", authorization);
        }
        let req = builder
            .body(Body::from(serde_json::json!({ "text": "hello" }).to_string()))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({}));
        (status, json)
    }

    async fn get(router: axum::Router, uri: &str) -> StatusCode {
        let req = Request::builder().method(Method::GET).uri(uri).body(Body::empty()).unwrap();
        router.oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn agent_guard_missing_authorization_header_returns_identity_required() {
        let h = harness().await;

        let (status, body) = post_text(h.router, &h.surface_id, None).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentIdentityRequired);
        assert!(h.adapter.text_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_invalid_bearer_token_returns_identity_required() {
        let h = harness().await;

        let (status, body) = post_text(h.router, &h.surface_id, Some("pta_agent_missing.nope")).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentIdentityRequired);
        assert!(h.adapter.text_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_valid_identity_without_grant_creates_pending_request_and_event() {
        let mut h = harness().await;

        let (status, body) = post_text(h.router, &h.surface_id, Some(&h.token)).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.surface_id, Some(h.surface_id));
        assert_eq!(details.actions, vec![ActionClass::Drive]);
        assert!(h.store.get_permission_request(&details.request_id).await.unwrap().is_some());
        assert!(matches!(
            h.events.recv().await.unwrap(),
            AgentEvent::AgentPermissionRequested { .. }
        ));
        assert!(h.adapter.text_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_accepts_case_insensitive_bearer_scheme() {
        let h = harness().await;
        let (_, body) = post_text(h.router.clone(), &h.surface_id, Some(&h.token)).await;
        let err: WireError = serde_json::from_value(body).unwrap();
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        h.store
            .approve_request(&details.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), NOW + 1)
            .await
            .unwrap();

        let auth_header = format!("bearer {}", h.token);
        let (status, _) = post_text_with_auth_header(h.router, &h.surface_id, Some(&auth_header)).await;

        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn agent_guard_observe_route_requires_identity_before_adapter_call() {
        let h = harness().await;

        let (status, body) = post_json(
            h.router,
            format!("/surfaces/{}/screenshot", h.surface_id),
            None,
            serde_json::json!({}),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentIdentityRequired);
        assert!(h.adapter.screenshot_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_observe_route_creates_observe_request_before_adapter_call() {
        let h = harness().await;

        let (status, body) = post_json(
            h.router,
            format!("/surfaces/{}/screenshot", h.surface_id),
            Some(&h.token),
            serde_json::json!({}),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Observe]);
        assert_eq!(details.surface_id, Some(h.surface_id));
        assert!(h.adapter.screenshot_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_manage_route_creates_manage_request_before_adapter_call() {
        let h = harness().await;

        let (status, body) = post_json(
            h.router,
            format!("/surfaces/{}/place", h.surface_id),
            Some(&h.token),
            serde_json::json!({ "rect": { "x": 10.0, "y": 20.0, "w": 800.0, "h": 600.0 } }),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Manage]);
        assert_eq!(details.surface_id, Some(h.surface_id));
        assert!(h.adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_record_route_requires_observe_and_record_grant() {
        let h = harness().await;

        let (status, body) = post_json(
            h.router.clone(),
            format!("/capture-sessions/surfaces/{}", h.surface_id),
            Some(&h.token),
            serde_json::json!({}),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Observe, ActionClass::Record]);
        h.store
            .approve_request(&details.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), NOW + 1)
            .await
            .unwrap();

        let (status, body) = post_json(
            h.router,
            format!("/capture-sessions/surfaces/{}", h.surface_id),
            Some(&h.token),
            serde_json::json!({}),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let created: CreateCaptureSessionResponse = serde_json::from_value(body).unwrap();
        assert_eq!(created.track_id, 1);
    }

    #[tokio::test]
    async fn agent_guard_launch_route_creates_launched_by_agent_manage_request_before_adapter_call() {
        let h = harness().await;

        let (status, body) = post_json(
            h.router,
            "/launches",
            Some(&h.token),
            serde_json::json!({
                "kind": { "type": "process", "app": "test", "args": [] },
                "require_confidence": "strong"
            }),
        )
        .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Manage]);
        assert_eq!(details.target, AgentPermissionTarget::LaunchedByAgent);
        assert!(h.adapter.launch_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_search_route_creates_all_surfaces_manage_request_before_adapter_call() {
        let h = harness().await;

        let (status, body) = post_json(h.router, "/surfaces/search", Some(&h.token), serde_json::json!({})).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Manage]);
        assert_eq!(details.target, AgentPermissionTarget::AllSurfaces);
        assert!(h.adapter.search_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_attention_route_creates_all_surfaces_manage_request_before_adapter_call() {
        let h = harness().await;

        let (status, body) = get_json(h.router, "/attention", Some(&h.token)).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionNeeded);
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        assert_eq!(details.actions, vec![ActionClass::Manage]);
        assert_eq!(details.target, AgentPermissionTarget::AllSurfaces);
    }

    #[tokio::test]
    async fn agent_guard_reuses_pending_request_without_duplicate_event() {
        let mut h = harness().await;

        let (_, first_body) = post_text(h.router.clone(), &h.surface_id, Some(&h.token)).await;
        assert!(matches!(
            h.events.recv().await.unwrap(),
            AgentEvent::AgentPermissionRequested { .. }
        ));
        let (_, second_body) = post_text(h.router.clone(), &h.surface_id, Some(&h.token)).await;

        let first_err: WireError = serde_json::from_value(first_body).unwrap();
        let first_details: AgentPermissionNeededDetails = serde_json::from_value(first_err.details.unwrap()).unwrap();
        let second_err: WireError = serde_json::from_value(second_body).unwrap();
        let second_details: AgentPermissionNeededDetails = serde_json::from_value(second_err.details.unwrap()).unwrap();
        assert_eq!(second_details.request_id, first_details.request_id);
        assert!(matches!(
            h.events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn agent_guard_approval_allows_retry_of_same_text_call() {
        let h = harness().await;
        let (_, body) = post_text(h.router.clone(), &h.surface_id, Some(&h.token)).await;
        let err: WireError = serde_json::from_value(body).unwrap();
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        h.store
            .approve_request(&details.request_id, DurationSpec::UntilSurfaceGone, Vec::new(), NOW + 1)
            .await
            .unwrap();

        let (status, body) = post_text(h.router, &h.surface_id, Some(&h.token)).await;

        assert_eq!(status, StatusCode::OK);
        let resp: TextResponse = serde_json::from_value(body).unwrap();
        assert_eq!(resp.chars_sent, 5);
        assert_eq!(h.adapter.text_calls().await.len(), 1);
        let executed_at = h.store.debug_executed_audit_times().await.unwrap();
        assert_eq!(executed_at.len(), 1);
        assert!(executed_at[0] > 0);
    }

    #[tokio::test]
    async fn agent_guard_remembered_denial_blocks_retry() {
        let h = harness().await;
        let (_, body) = post_text(h.router.clone(), &h.surface_id, Some(&h.token)).await;
        let err: WireError = serde_json::from_value(body).unwrap();
        let details: AgentPermissionNeededDetails = serde_json::from_value(err.details.unwrap()).unwrap();
        h.store
            .deny_request(&details.request_id, true, Some("no".into()), NOW + 1)
            .await
            .unwrap();

        let (status, body) = post_text(h.router, &h.surface_id, Some(&h.token)).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::AgentPermissionDenied);
        assert!(h.adapter.text_calls().await.is_empty());
    }

    #[tokio::test]
    async fn agent_guard_info_remains_unauthenticated() {
        let h = harness().await;

        let status = get(h.router, "/info").await;

        assert_eq!(status, StatusCode::OK);
    }
}
