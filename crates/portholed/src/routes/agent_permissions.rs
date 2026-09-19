use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::{
    ErrorCode, PortholeError,
    agent_policy::{AgentId, DurationSpec, GrantId, PermissionRequestId, TargetSelector},
};
use porthole_protocol::agent_permissions::{
    AgentGrantResponse, AgentIdentityResponse, AgentPermissionConstraints, AgentPermissionRequestResponse, ApproveAgentPermissionRequest,
    CreateAgentIdentityRequest, CreateAgentIdentityResponse, DenyAgentPermissionRequest, MintAgentTokenResponse, PermissionDescription,
    PermissionRequestContext, PermissionSurface, RevocationResponse,
};

use crate::{
    agent_store::{PermissionRequestStatus, StoredAgentIdentity, StoredGrant, StoredPermissionRequest},
    events::AgentEvent,
    routes::errors::ApiError,
    state::AppState,
};

pub async fn post_identity(
    State(state): State<AppState>,
    Json(request): Json<CreateAgentIdentityRequest>,
) -> Result<Json<CreateAgentIdentityResponse>, ApiError> {
    let created = state
        .agent_store
        .create_identity(request.display_name.clone(), request.metadata, now_unix_ms())
        .await?;
    state.events.publish(AgentEvent::AgentIdentityCreated {
        agent_id: created.agent_id.clone(),
        display_name: request.display_name,
    });
    Ok(Json(CreateAgentIdentityResponse {
        agent_id: created.agent_id,
        token: created.token,
    }))
}

pub async fn get_identities(State(state): State<AppState>) -> Result<Json<Vec<AgentIdentityResponse>>, ApiError> {
    let identities = state.agent_store.list_identities().await?;
    Ok(Json(identities.into_iter().map(identity_response).collect()))
}

pub async fn get_identity(State(state): State<AppState>, Path(agent_id): Path<String>) -> Result<Json<AgentIdentityResponse>, ApiError> {
    let Some(identity) = state.agent_store.get_identity(&AgentId::from(agent_id)).await? else {
        return Err(not_found("agent identity not found"));
    };
    Ok(Json(identity_response(identity)))
}

pub async fn post_revoke_identity(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<RevocationResponse>, ApiError> {
    let agent_id = AgentId::from(agent_id);
    let revoked = state.agent_store.revoke_identity(&agent_id, now_unix_ms()).await?;
    if revoked {
        state.events.publish(AgentEvent::AgentIdentityRevoked { agent_id });
    }
    Ok(Json(RevocationResponse { revoked }))
}

pub async fn post_identity_token(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<MintAgentTokenResponse>, ApiError> {
    let created = state.agent_store.mint_token(&AgentId::from(agent_id), now_unix_ms()).await?;
    state.events.publish(AgentEvent::AgentPolicyChanged {
        resource: "agent_token".into(),
    });
    Ok(Json(MintAgentTokenResponse {
        token_id: created.token_id,
        token: created.token,
    }))
}

pub async fn post_revoke_identity_token(
    State(state): State<AppState>,
    Path((agent_id, token_id)): Path<(String, String)>,
) -> Result<Json<RevocationResponse>, ApiError> {
    let revoked = state
        .agent_store
        .revoke_token(&AgentId::from(agent_id), &token_id, now_unix_ms())
        .await?;
    if revoked {
        state.events.publish(AgentEvent::AgentPolicyChanged {
            resource: "agent_token".into(),
        });
    }
    Ok(Json(RevocationResponse { revoked }))
}

pub async fn get_requests(State(state): State<AppState>) -> Result<Json<Vec<AgentPermissionRequestResponse>>, ApiError> {
    let requests = state.agent_store.list_pending_permission_requests().await?;
    let mut result = Vec::with_capacity(requests.len());
    for request in requests {
        let response = request_response(&state, request).await?;
        if response.status == "pending" {
            result.push(response);
        }
    }
    Ok(Json(result))
}

pub async fn get_request(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Result<Json<AgentPermissionRequestResponse>, ApiError> {
    let request_id = PermissionRequestId::from(request_id);
    let Some(request) = state.agent_store.get_permission_request(&request_id).await? else {
        return Err(ApiError::from(crate::agent_store::AgentStoreError::PermissionRequestNotFound));
    };
    Ok(Json(request_response(&state, request).await?))
}

pub async fn post_approve_request(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(body): Json<ApproveAgentPermissionRequest>,
) -> Result<Json<AgentGrantResponse>, ApiError> {
    let request_id = PermissionRequestId::from(request_id);
    let Some(request) = state.agent_store.get_permission_request(&request_id).await? else {
        return Err(ApiError::from(crate::agent_store::AgentStoreError::PermissionRequestNotFound));
    };
    if request_response(&state, request.clone()).await?.status != "pending" {
        return Err(ApiError::from(crate::agent_store::AgentStoreError::PermissionRequestNotPending));
    }
    let body_target: TargetSelector = body.target.into();
    let mut body_actions = body.actions;
    body_actions.sort();
    body_actions.dedup();
    if request.target != body_target || request.actions != body_actions {
        return Err(ApiError::from(PortholeError::new(
            ErrorCode::InvalidArgument,
            "approve target/actions must match the pending request",
        )));
    }
    let duration: DurationSpec = body.duration.into();
    let constraints: Vec<porthole_core::agent_policy::Constraint> = body.constraints.into();
    let grant = state
        .agent_store
        .approve_request(&request_id, duration, constraints, now_unix_ms())
        .await?;
    state.events.publish(AgentEvent::AgentPermissionResolved {
        request_id,
        status: "approved".into(),
    });
    Ok(Json(grant_response(&state, grant).await?))
}

pub async fn post_deny_request(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
    Json(body): Json<DenyAgentPermissionRequest>,
) -> Result<Json<AgentPermissionRequestResponse>, ApiError> {
    let request_id = PermissionRequestId::from(request_id);
    let request = state
        .agent_store
        .deny_request(&request_id, body.remember, body.reason, now_unix_ms())
        .await?;
    state.events.publish(AgentEvent::AgentPermissionResolved {
        request_id,
        status: "denied".into(),
    });
    Ok(Json(request_response(&state, request).await?))
}

pub async fn get_grants(State(state): State<AppState>) -> Result<Json<Vec<AgentGrantResponse>>, ApiError> {
    let grants = state.agent_store.list_active_grants(now_unix_ms()).await?;
    let mut result = Vec::with_capacity(grants.len());
    for grant in grants {
        let response = grant_response(&state, grant).await?;
        // This endpoint lists effective grants for the live inbox, not history.
        // Identity revocation disables its grants without deleting their records.
        if response.description.agent_revoked
            || (matches!(
                response.duration,
                porthole_protocol::agent_permissions::AgentPermissionDuration::UntilSurfaceGone
            ) && response.description.surface_available == Some(false))
        {
            continue;
        }
        result.push(response);
    }
    Ok(Json(result))
}

pub async fn post_revoke_grant(State(state): State<AppState>, Path(grant_id): Path<String>) -> Result<Json<RevocationResponse>, ApiError> {
    let revoked = state.agent_store.revoke_grant(&GrantId::from(grant_id), now_unix_ms()).await?;
    if revoked {
        state.events.publish(AgentEvent::AgentPolicyChanged {
            resource: "agent_grant".into(),
        });
    }
    Ok(Json(RevocationResponse { revoked }))
}

fn identity_response(identity: StoredAgentIdentity) -> AgentIdentityResponse {
    AgentIdentityResponse {
        agent_id: identity.agent_id,
        display_name: identity.display_name,
        metadata: identity.metadata,
        created_at_unix_ms: identity.created_at_unix_ms,
        revoked_at_unix_ms: identity.revoked_at_unix_ms,
    }
}

async fn request_response(state: &AppState, mut request: StoredPermissionRequest) -> Result<AgentPermissionRequestResponse, ApiError> {
    let description = describe(state, &request.agent_id, &request.target, request.context.clone()).await?;
    // Missing/dead handle IDs and revoked identities cannot become valid again.
    // Only successful reads of local state reach here; connection/OS failures
    // are never grounds for retiring a pending request.
    if request.status == PermissionRequestStatus::Pending {
        let reason = if description.agent_revoked {
            Some("requester_unavailable")
        } else if description.surface_available == Some(false) {
            Some("surface_unavailable")
        } else {
            None
        };
        if let Some(reason) = reason {
            let (updated, changed) = state
                .agent_store
                .invalidate_request(&request.request_id, reason, now_unix_ms())
                .await?;
            request = updated;
            if changed {
                state.events.publish(AgentEvent::AgentPermissionResolved {
                    request_id: request.request_id.clone(),
                    status: "invalidated".into(),
                });
            }
        }
    }
    Ok(AgentPermissionRequestResponse {
        description,
        request_id: request.request_id,
        agent_id: request.agent_id,
        target: request.target.into(),
        actions: request.actions,
        reason: request.reason,
        status: request.status.as_str().to_string(),
        invalidation_reason: request.invalidation_reason,
        created_at_unix_ms: request.created_at_unix_ms,
        resolved_at_unix_ms: request.resolved_at_unix_ms,
    })
}

async fn grant_response(state: &AppState, grant: StoredGrant) -> Result<AgentGrantResponse, ApiError> {
    let (context, origin_reason) = match &grant.origin_request_id {
        Some(id) => state
            .agent_store
            .get_permission_request(id)
            .await?
            .map(|r| (r.context, r.reason))
            .unwrap_or_default(),
        None => (PermissionRequestContext::default(), None),
    };
    let description = describe(state, &grant.agent_id, &grant.target, context).await?;
    Ok(AgentGrantResponse {
        description,
        grant_id: grant.grant_id,
        agent_id: grant.agent_id,
        origin_request_id: grant.origin_request_id,
        origin_reason,
        target: grant.target.into(),
        actions: grant.actions,
        duration: grant.duration.into(),
        constraints: AgentPermissionConstraints::from(grant.constraints),
        created_at_unix_ms: grant.created_at_unix_ms,
        expires_at_unix_ms: grant.expires_at_unix_ms,
        consumed_at_unix_ms: grant.consumed_at_unix_ms,
        revoked_at_unix_ms: grant.revoked_at_unix_ms,
    })
}

fn not_found(message: &str) -> ApiError {
    ApiError::from(PortholeError::new(ErrorCode::AgentIdentityNotFound, message))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before Unix epoch")
        .as_millis() as u64
}

async fn describe(
    state: &AppState,
    agent: &AgentId,
    target: &TargetSelector,
    context: PermissionRequestContext,
) -> Result<PermissionDescription, ApiError> {
    let identity = state.agent_store.get_identity(agent).await?;
    let (surface, surface_available) = match target {
        TargetSelector::Surface { surface_id } | TargetSelector::FrontmostOnce { surface_id } => {
            let current = state.handles.get(surface_id).await.ok();
            let available = current
                .as_ref()
                .is_some_and(|s| s.state == porthole_core::surface::SurfaceState::Alive);
            let surface = context.surface.or_else(|| {
                current.map(|s| PermissionSurface {
                    app_name: s.app_name,
                    title: s.title,
                    pid: s.pid,
                })
            });
            (surface, Some(available))
        }
        _ => (context.surface, None),
    };
    Ok(PermissionDescription {
        agent_name: identity.as_ref().map(|i| i.display_name.clone()),
        agent_revoked: identity.is_none_or(|i| i.revoked_at_unix_ms.is_some()),
        surface,
        surface_available,
        operation: context.operation,
    })
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
        agent_policy::{ActionClass, TargetSelector},
        in_memory::InMemoryAdapter,
    };
    use porthole_protocol::{
        agent_permissions::{
            AgentPermissionDuration, AgentPermissionRequestResponse, AgentPermissionTarget, ApproveAgentPermissionRequest,
            CreateAgentIdentityResponse, DenyAgentPermissionRequest, RevocationResponse,
        },
        error::WireError,
    };
    use tower::ServiceExt;

    use crate::{
        agent_store::AgentPolicyStore,
        events::{AgentEvent, EventBus},
        server::build_router,
        state::AppState,
    };

    const NOW: u64 = 1_000;

    async fn test_state() -> AppState {
        let state = AppState::new_with_agent_policy(
            Arc::new(InMemoryAdapter::new()),
            AgentPolicyStore::open_in_memory().await.unwrap(),
            EventBus::new(),
        );
        let mut surface = porthole_core::surface::SurfaceInfo::window(SurfaceId::from("surf_1"), 42);
        surface.app_name = Some("Kitty".into());
        surface.title = Some("Project terminal".into());
        state.handles.insert(surface).await;
        state
    }

    #[tokio::test]
    async fn request_descriptions_retain_first_operation_without_text_and_follow_grants() {
        let state = test_state().await;
        let identity = state.agent_store.create_identity("KS presenter", None, NOW).await.unwrap();
        let router = build_router(state.clone());
        for text in ["秘密🐈", "different retry"] {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/surfaces/surf_1/text")
                .header("authorization", format!("Bearer {}", identity.token))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"text": text}).to_string()))
                .unwrap();
            assert_eq!(router.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);
        }
        let (_, requests) = get(router.clone(), "/agent-permissions/requests").await;
        assert_eq!(requests.as_array().unwrap().len(), 1);
        let description = &requests[0]["description"];
        assert_eq!(description["agent_name"], "KS presenter");
        assert_eq!(description["surface"]["app_name"], "Kitty");
        assert_eq!(description["surface"]["title"], "Project terminal");
        assert_eq!(description["operation"], serde_json::json!({"kind":"text", "characters":3}));
        assert!(!requests.to_string().contains("秘密"));
        assert!(!requests.to_string().contains("different retry"));
        let id = requests[0]["request_id"].as_str().unwrap();
        let (status, grant) = post(
            router.clone(),
            &format!("/agent-permissions/requests/{id}/approve"),
            serde_json::json!({
                "duration":{"type":"until_surface_gone"}, "target":surface_target("surf_1"), "actions":["drive"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{grant}");
        assert_eq!(grant["description"], *description);
        state.handles.mark_dead(&SurfaceId::from("surf_1")).await.unwrap();
        let (_, old) = get(router.clone(), &format!("/agent-permissions/requests/{id}")).await;
        assert_eq!(old["description"]["surface_available"], false);
        assert_eq!(old["description"]["surface"]["title"], "Project terminal");
        let (_, grants) = get(router, "/agent-permissions/grants").await;
        assert!(grants.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn inbox_retires_unavailable_windows_and_revoked_requesters_but_keeps_history() {
        let state = test_state().await;
        let agent = state.agent_store.create_identity("stale requester", None, NOW).await.unwrap();
        let mut ids = Vec::new();
        for surface in ["surf_1", "surf_from_previous_daemon"] {
            let request = state
                .agent_store
                .create_pending_request(
                    agent.agent_id.clone(),
                    TargetSelector::Surface {
                        surface_id: surface.into(),
                    },
                    vec![ActionClass::Observe],
                    Some("inspect window".into()),
                    NOW,
                )
                .await
                .unwrap();
            ids.push(request.request_id);
        }
        let broad = state
            .agent_store
            .create_pending_request(
                agent.agent_id.clone(),
                TargetSelector::AllSurfaces,
                vec![ActionClass::Observe],
                Some("search surfaces".into()),
                NOW,
            )
            .await
            .unwrap();
        state.handles.mark_dead(&SurfaceId::from("surf_1")).await.unwrap();
        let mut events = state.events.subscribe();
        let router = build_router(state.clone());
        let (_, pending) = get(router.clone(), "/agent-permissions/requests").await;
        assert_eq!(pending.as_array().unwrap().len(), 1);
        assert_eq!(pending[0]["request_id"], broad.request_id.to_string());
        for id in ids {
            let (_, detail) = get(router.clone(), &format!("/agent-permissions/requests/{id}")).await;
            assert_eq!(detail["status"], "invalidated");
            assert_eq!(detail["invalidation_reason"], "surface_unavailable");
            assert_eq!(detail["reason"], "inspect window");
            assert!(detail["resolved_at_unix_ms"].is_number());
            assert!(matches!(events.try_recv().unwrap(), AgentEvent::AgentPermissionResolved { status, .. } if status == "invalidated"));
        }
        assert!(events.try_recv().is_err());
        state.agent_store.revoke_identity(&agent.agent_id, NOW + 1).await.unwrap();
        let (_, pending) = get(router.clone(), "/agent-permissions/requests").await;
        assert!(pending.as_array().unwrap().is_empty());
        let (_, detail) = get(router, &format!("/agent-permissions/requests/{}", broad.request_id)).await;
        assert_eq!(detail["status"], "invalidated");
        assert_eq!(detail["invalidation_reason"], "requester_unavailable");
        assert!(matches!(events.try_recv().unwrap(), AgentEvent::AgentPermissionResolved { status, .. } if status == "invalidated"));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn direct_approval_retires_missing_window_requests_without_creating_a_grant() {
        let state = test_state().await;
        let agent = state.agent_store.create_identity("old requester", None, NOW).await.unwrap();
        let pending = state
            .agent_store
            .create_pending_request(
                agent.agent_id,
                TargetSelector::Surface {
                    surface_id: "missing".into(),
                },
                vec![ActionClass::Observe],
                None,
                NOW,
            )
            .await
            .unwrap();
        let router = build_router(state.clone());
        let (status, _) = post(
            router.clone(),
            &format!("/agent-permissions/requests/{}/approve", pending.request_id),
            serde_json::json!({"duration":{"type":"once"},"target":surface_target("missing"),"actions":["observe"]}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(state.agent_store.list_active_grants(NOW).await.unwrap().is_empty());
        let (_, detail) = get(router, &format!("/agent-permissions/requests/{}", pending.request_id)).await;
        assert_eq!(detail["status"], "invalidated");
    }

    async fn request(router: axum::Router, method: Method, uri: &str, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(body) => {
                builder = builder.header("content-type", "application/json");
                Body::from(body.to_string())
            }
            None => Body::empty(),
        };
        let res = router.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        let json = if bytes.is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    async fn post(router: axum::Router, uri: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        request(router, Method::POST, uri, Some(body)).await
    }

    async fn get(router: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
        request(router, Method::GET, uri, None).await
    }

    fn surface_target(surface_id: &str) -> serde_json::Value {
        serde_json::json!({ "type": "surface", "surface_id": surface_id })
    }

    #[tokio::test]
    async fn agent_permission_routes_create_list_show_mint_and_revoke_identity_without_leaking_tokens() {
        let state = test_state().await;
        let mut events = state.events.subscribe();
        let router = build_router(state);

        let (status, body) = post(
            router.clone(),
            "/agent-identities",
            serde_json::json!({
                "display_name": "Build Agent",
                "metadata": { "bundle_id": "com.example.agent", "vendor": "Example" }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let created: CreateAgentIdentityResponse = serde_json::from_value(body).unwrap();
        assert!(created.token.starts_with(&format!("pta_{}.", created.agent_id)));
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentIdentityCreated { .. }));

        let (status, identities) = get(router.clone(), "/agent-identities").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(identities.as_array().unwrap().len(), 1);
        assert_eq!(identities[0]["display_name"], "Build Agent");
        assert!(identities.to_string().contains("com.example.agent"));
        assert!(!identities.to_string().contains(&created.token));

        let (status, identity) = get(router.clone(), &format!("/agent-identities/{}", created.agent_id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(identity["agent_id"], created.agent_id.to_string());

        let (status, missing) = get(router.clone(), "/agent-identities/agent_missing").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let missing: WireError = serde_json::from_value(missing).unwrap();
        assert_eq!(missing.code, ErrorCode::AgentIdentityNotFound);

        let (status, token) = post(
            router.clone(),
            &format!("/agent-identities/{}/tokens", created.agent_id),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(token["token"].as_str().unwrap().starts_with(&format!("pta_{}.", created.agent_id)));
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentPolicyChanged { .. }));

        let token_id = token["token_id"].as_str().unwrap();
        let (status, revoked) = post(
            router.clone(),
            &format!("/agent-identities/{}/tokens/{token_id}/revoke", created.agent_id),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(serde_json::from_value::<RevocationResponse>(revoked).unwrap().revoked);
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentPolicyChanged { .. }));

        let (status, revoked) = post(
            router,
            &format!("/agent-identities/{}/revoke", created.agent_id),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(serde_json::from_value::<RevocationResponse>(revoked).unwrap().revoked);
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentIdentityRevoked { .. }));
    }

    #[tokio::test]
    async fn agent_permission_routes_list_approve_list_grants_and_revoke_with_events() {
        let state = test_state().await;
        let store = state.agent_store.clone();
        let mut events = state.events.subscribe();
        let identity = store.create_identity("agent", None, NOW).await.unwrap();
        let pending = store
            .create_pending_request(
                identity.agent_id.clone(),
                TargetSelector::Surface {
                    surface_id: SurfaceId::from("surf_1"),
                },
                vec![ActionClass::Drive],
                Some("typing".into()),
                NOW + 1,
            )
            .await
            .unwrap();
        let router = build_router(state);

        let (status, requests) = get(router.clone(), "/agent-permissions/requests").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(requests.as_array().unwrap().len(), 1);
        let listed: AgentPermissionRequestResponse = serde_json::from_value(requests[0].clone()).unwrap();
        assert_eq!(listed.request_id, pending.request_id);
        assert_eq!(listed.reason.as_deref(), Some("typing"));

        let (status, detail) = get(router.clone(), &format!("/agent-permissions/requests/{}", pending.request_id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["request_id"], pending.request_id.to_string());

        let (status, grant) = post(
            router.clone(),
            &format!("/agent-permissions/requests/{}/approve", pending.request_id),
            serde_json::to_value(ApproveAgentPermissionRequest {
                duration: AgentPermissionDuration::UntilSurfaceGone,
                target: AgentPermissionTarget::Surface {
                    surface_id: SurfaceId::from("surf_1"),
                },
                actions: vec![ActionClass::Drive],
                constraints: Default::default(),
            })
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let grant_id = grant["grant_id"].as_str().unwrap().to_string();
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentPermissionResolved { .. }));

        let (status, grants) = get(router.clone(), "/agent-permissions/grants").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(grants.as_array().unwrap().len(), 1);
        assert_eq!(grants[0]["origin_request_id"], pending.request_id.to_string());
        assert_eq!(grants[0]["origin_reason"], "typing");

        let (status, revoked) = post(
            router.clone(),
            &format!("/agent-permissions/grants/{grant_id}/revoke"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(serde_json::from_value::<RevocationResponse>(revoked).unwrap().revoked);
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentPolicyChanged { .. }));

        let (status, grants) = get(router, "/agent-permissions/grants").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(grants.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn agent_permission_routes_approve_rejects_target_or_action_mismatch_without_creating_grant() {
        let state = test_state().await;
        let store = state.agent_store.clone();
        let identity = store.create_identity("agent", None, NOW).await.unwrap();
        let pending = store
            .create_pending_request(
                identity.agent_id,
                TargetSelector::Surface {
                    surface_id: SurfaceId::from("surf_1"),
                },
                vec![ActionClass::Drive],
                None,
                NOW + 1,
            )
            .await
            .unwrap();
        let router = build_router(state);

        let (status, body) = post(
            router.clone(),
            &format!("/agent-permissions/requests/{}/approve", pending.request_id),
            serde_json::json!({
                "duration": { "type": "until_surface_gone" },
                "target": surface_target("surf_2"),
                "actions": ["drive"],
                "constraints": { "requires_frontmost": false }
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let err: WireError = serde_json::from_value(body).unwrap();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        let (status, grants) = get(router, "/agent-permissions/grants").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(grants.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn agent_permission_routes_deny_request_resolves_and_remembered_denial_is_listed_by_snapshot() {
        let state = test_state().await;
        let store = state.agent_store.clone();
        let mut events = state.events.subscribe();
        let identity = store.create_identity("agent", None, NOW).await.unwrap();
        let pending = store
            .create_pending_request(
                identity.agent_id.clone(),
                TargetSelector::Surface {
                    surface_id: SurfaceId::from("surf_1"),
                },
                vec![ActionClass::Drive],
                None,
                NOW + 1,
            )
            .await
            .unwrap();
        let router = build_router(state);

        let (status, body) = post(
            router,
            &format!("/agent-permissions/requests/{}/deny", pending.request_id),
            serde_json::to_value(DenyAgentPermissionRequest {
                remember: true,
                reason: Some("not_now".into()),
            })
            .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "denied");
        assert!(matches!(events.recv().await.unwrap(), AgentEvent::AgentPermissionResolved { .. }));
        let snapshot = store.load_policy_snapshot(&identity.agent_id).await.unwrap();
        assert_eq!(snapshot.denials.len(), 1);
    }
}
