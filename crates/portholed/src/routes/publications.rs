//! Publications, exports and republications.
//!
//! `GET /publications` lists every capture session and republication as a
//! publication. `POST /publications/{id}/exports` spawns the producer-side
//! bridge half for a native capture session; exports are reserved to the
//! session's owner, like output sizing. `POST /publications/republish` creates
//! the consumer-side half from forwarded sockets and reports a new native
//! publication owned by the caller. Export responses carry the link token, so
//! nothing here is unauthenticated.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use jackstay_graph::Identities;
use porthole_core::error::{ErrorCode, PortholeError};
use porthole_protocol::publications::{
    CreateExportRequest, ExportResponse, ListPublicationsResponse, PUBLICATION_KIND_CAPTURE, PublicationResponse, RepublishRequest,
    RepublishResponse,
};

use crate::{
    capture_registry::CaptureRegistryError,
    export_registry::ExportError,
    routes::{agent_guard::authenticated_agent_id, capture_sessions::capture_error_to_api, errors::ApiError},
    state::AppState,
};

fn export_error_to_api(error: ExportError) -> ApiError {
    let code = match &error {
        ExportError::UnknownExport(_) | ExportError::UnknownRepublication(_) => ErrorCode::SurfaceNotFound,
        ExportError::NotOwner => ErrorCode::AgentPermissionDenied,
        ExportError::BridgeMissing | ExportError::Unsupported => ErrorCode::AdapterUnsupported,
        ExportError::Io(_) | ExportError::Token(_) | ExportError::Spawn(_) | ExportError::Poisoned | ExportError::RepublishFailed(_) => {
            ErrorCode::InternalError
        }
    };
    ApiError(PortholeError::new(code, error.to_string()).into())
}

fn capture_publication(session: &porthole_protocol::capture_sessions::CaptureSessionResponse, source: String) -> PublicationResponse {
    PublicationResponse {
        publication_id: session.session_id.clone(),
        kind: PUBLICATION_KIND_CAPTURE.to_owned(),
        identities: Identities {
            source,
            publication: session.session_id.clone(),
        },
        status: session.status.clone(),
        status_message: session.status_message.clone(),
        width: session.width,
        height: session.height,
        native: session.native.clone(),
    }
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<ListPublicationsResponse>, ApiError> {
    let _agent = authenticated_agent_id(&state, &headers).await?;
    let mut publications: Vec<PublicationResponse> = state
        .capture
        .list_sessions()
        .map_err(capture_error_to_api)?
        .into_iter()
        .map(|(session, source)| capture_publication(&session, source))
        .collect();
    publications.extend(state.exports.list_republications());
    Ok(Json(ListPublicationsResponse { publications }))
}

pub async fn get(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<PublicationResponse>, ApiError> {
    let _agent = authenticated_agent_id(&state, &headers).await?;
    if let Some(publication) = state.exports.republication(&id) {
        return Ok(Json(publication));
    }
    let (session, source) = state.capture.session_with_source(&id).map_err(capture_error_to_api)?;
    Ok(Json(capture_publication(&session, source)))
}

pub async fn post_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<CreateExportRequest>,
) -> Result<(StatusCode, Json<ExportResponse>), ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    let (session, source) = state.capture.session_with_source(&id).map_err(capture_error_to_api)?;
    let owner = state.capture.session_owner(&id).map_err(capture_error_to_api)?;
    if owner.as_ref().is_some_and(|owner| owner != &agent_id) {
        return Err(capture_error_to_api(CaptureRegistryError::from_porthole(PortholeError::new(
            ErrorCode::AgentPermissionDenied,
            "only the capture session owner may export it",
        ))));
    }
    let Some(native) = session.native.as_ref() else {
        return Err(ApiError(
            PortholeError::new(ErrorCode::InvalidArgument, "only native capture sessions can be exported").into(),
        ));
    };
    let identities = Identities {
        source,
        publication: id.clone(),
    };
    let exports = state.exports.clone();
    let native = native.clone();
    // Spawning waits for the half to bind its sockets; keep that off the runtime.
    let response =
        tokio::task::spawn_blocking(move || exports.create_export(&id, agent_id, identities, &native, request.chroma, request.bitrate_bps))
            .await
            .map_err(|e| ApiError(PortholeError::new(ErrorCode::InternalError, format!("export task: {e}")).into()))?
            .map_err(export_error_to_api)?;
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn get_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, export_id)): Path<(String, String)>,
) -> Result<Json<ExportResponse>, ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    Ok(Json(
        state
            .exports
            .export_status(&id, &export_id, &agent_id)
            .map_err(export_error_to_api)?,
    ))
}

pub async fn delete_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, export_id)): Path<(String, String)>,
) -> Result<Json<ExportResponse>, ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    Ok(Json(
        state
            .exports
            .close_export(&id, &export_id, &agent_id)
            .map_err(export_error_to_api)?,
    ))
}

pub async fn post_republish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RepublishRequest>,
) -> Result<(StatusCode, Json<RepublishResponse>), ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    let exports = state.exports.clone();
    let response = tokio::task::spawn_blocking(move || {
        exports.republish(
            agent_id,
            request.identities,
            std::path::Path::new(&request.media_socket),
            std::path::Path::new(&request.control_socket),
            request.link_token,
            request.chroma,
        )
    })
    .await
    .map_err(|e| ApiError(PortholeError::new(ErrorCode::InternalError, format!("republish task: {e}")).into()))?
    .map_err(export_error_to_api)?;
    Ok((StatusCode::CREATED, Json(response)))
}

pub async fn delete_republication(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    state.exports.close_republication(&id, &agent_id).map_err(export_error_to_api)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use porthole_core::{ErrorCode, in_memory::InMemoryAdapter};
    use porthole_protocol::{capture_sessions::NativeCaptureInfo, error::WireError};
    use tower::ServiceExt;

    use crate::{
        agent_store::AgentPolicyStore, capture_registry::CaptureRegistry, events::EventBus, export_registry::ExportRegistry,
        server::build_router, state::AppState,
    };

    struct Harness {
        router: axum::Router,
        owner_token: String,
        other_token: String,
        _temp: tempfile::TempDir,
    }

    async fn harness() -> Harness {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentPolicyStore::open_in_memory().await.unwrap();
        let capture = CaptureRegistry::with_fd_socket(temp.path().join("capture-transfer.sock")).unwrap();
        let owner = store.create_identity("owner", None, 1_000).await.unwrap();
        let other = store.create_identity("other", None, 1_000).await.unwrap();
        capture.insert_test_session(
            "sess_native",
            Some(owner.agent_id.clone()),
            Some("surf_1".to_owned()),
            Some(NativeCaptureInfo {
                transport_kind: 1,
                endpoint: "work.flotilla.porthole.attach".to_owned(),
                attach_token: "ptas_test".to_owned(),
            }),
        );
        // A CPU session is served over the fd socket, which Windows lacks.
        #[cfg(unix)]
        capture.insert_test_session("sess_cpu", None, None, None);
        let state = AppState::new_with_agent_policy_and_capture(Arc::new(InMemoryAdapter::new()), capture, store, EventBus::new())
            .with_exports(ExportRegistry::new(Some(temp.path().to_path_buf())));
        Harness {
            router: build_router(state),
            owner_token: owner.token,
            other_token: other.token,
            _temp: temp,
        }
    }

    async fn call(
        router: axum::Router,
        method: Method,
        uri: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
        let res = router.oneshot(builder.body(body).unwrap()).await.unwrap();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_else(|_| serde_json::json!({})))
    }

    fn error_code(json: &serde_json::Value) -> Option<ErrorCode> {
        serde_json::from_value::<WireError>(json.clone()).ok().map(|e| e.code)
    }

    #[tokio::test]
    async fn listing_requires_an_agent_and_merges_captures() {
        let h = harness().await;
        let (status, _) = call(h.router.clone(), Method::GET, "/publications", None, None).await;
        assert_ne!(status, StatusCode::OK);
        let (status, json) = call(h.router.clone(), Method::GET, "/publications", Some(&h.owner_token), None).await;
        assert_eq!(status, StatusCode::OK, "{json}");
        let publications = json["publications"].as_array().unwrap();
        assert_eq!(publications.len(), if cfg!(unix) { 2 } else { 1 });
        let native = publications.iter().find(|p| p["publication_id"] == "sess_native").unwrap();
        assert_eq!(native["kind"], "capture");
        assert_eq!(native["identities"]["source"], "surf_1");
        assert_eq!(native["identities"]["publication"], "sess_native");
        assert_eq!(native["native"]["endpoint"], "work.flotilla.porthole.attach");
        #[cfg(unix)]
        {
            let cpu = publications.iter().find(|p| p["publication_id"] == "sess_cpu").unwrap();
            assert_eq!(
                cpu["identities"]["source"], "sess_cpu",
                "a session without a surface is its own source"
            );
            assert!(cpu.get("native").is_none());
        }
    }

    #[tokio::test]
    async fn only_the_owner_may_export_and_only_native_sessions_qualify() {
        let h = harness().await;
        let (status, json) = call(
            h.router.clone(),
            Method::POST,
            "/publications/sess_native/exports",
            Some(&h.other_token),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(error_code(&json), Some(ErrorCode::AgentPermissionDenied), "{status} {json}");
        // The owner passes the ownership check; without a bridge binary the
        // registry then reports it unsupported, and with one it would spawn.
        let (status, json) = call(
            h.router.clone(),
            Method::POST,
            "/publications/sess_native/exports",
            Some(&h.owner_token),
            Some(serde_json::json!({})),
        )
        .await;
        assert_ne!(error_code(&json), Some(ErrorCode::AgentPermissionDenied), "{status} {json}");
        assert!(
            status == StatusCode::CREATED || error_code(&json) == Some(ErrorCode::AdapterUnsupported),
            "{status} {json}"
        );
        #[cfg(unix)]
        {
            let (_, json) = call(
                h.router.clone(),
                Method::POST,
                "/publications/sess_cpu/exports",
                Some(&h.owner_token),
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(error_code(&json), Some(ErrorCode::InvalidArgument), "{json}");
        }
    }

    #[tokio::test]
    async fn unknown_exports_and_republications_are_not_found() {
        let h = harness().await;
        let (_, json) = call(
            h.router.clone(),
            Method::GET,
            "/publications/sess_native/exports/exp_missing",
            Some(&h.owner_token),
            None,
        )
        .await;
        assert_eq!(error_code(&json), Some(ErrorCode::SurfaceNotFound), "{json}");
        let (_, json) = call(
            h.router.clone(),
            Method::DELETE,
            "/publications/rep_missing",
            Some(&h.owner_token),
            None,
        )
        .await;
        assert_eq!(error_code(&json), Some(ErrorCode::SurfaceNotFound), "{json}");
        let (_, json) = call(
            h.router.clone(),
            Method::GET,
            "/publications/sess_missing",
            Some(&h.owner_token),
            None,
        )
        .await;
        assert_eq!(error_code(&json), Some(ErrorCode::SurfaceNotFound), "{json}");
    }
}
