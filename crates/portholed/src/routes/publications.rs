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
        ExportError::Io(_) | ExportError::Poisoned | ExportError::RepublishFailed(_) => ErrorCode::InternalError,
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
    Path((_id, export_id)): Path<(String, String)>,
) -> Result<Json<ExportResponse>, ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    Ok(Json(
        state.exports.export_status(&export_id, &agent_id).map_err(export_error_to_api)?,
    ))
}

pub async fn delete_export(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_id, export_id)): Path<(String, String)>,
) -> Result<Json<ExportResponse>, ApiError> {
    let agent_id = authenticated_agent_id(&state, &headers).await?;
    Ok(Json(
        state.exports.close_export(&export_id, &agent_id).map_err(export_error_to_api)?,
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
