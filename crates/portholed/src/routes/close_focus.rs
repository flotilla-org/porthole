use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::close_focus::{CloseRequest, CloseResponse, FocusRequest, FocusResponse};

use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_close(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(_req): Json<CloseRequest>,
) -> Result<Json<CloseResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Manage], Some("close surface")).await?;
    state.input.close(&surface_id).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/close").await?;
    Ok(Json(CloseResponse {
        surface_id: surface_id.to_string(),
        closed: true,
    }))
}

pub async fn post_focus(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(_req): Json<FocusRequest>,
) -> Result<Json<FocusResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Drive], Some("focus surface")).await?;
    state.input.focus(&surface_id).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/focus").await?;
    Ok(Json(FocusResponse {
        surface_id: surface_id.to_string(),
        focused: true,
    }))
}
