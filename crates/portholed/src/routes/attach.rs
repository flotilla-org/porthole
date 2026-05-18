use axum::{Json, extract::State, http::HeaderMap};
use porthole_core::agent_policy::ActionClass;
use porthole_protocol::search::{SearchRequest, SearchResponse, TrackRequest, TrackResponse};

use crate::{
    routes::{
        agent_guard::{authorize_all_surfaces_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let execution = authorize_all_surfaces_actions(&state, &headers, &[ActionClass::Manage], Some("search surfaces")).await?;
    let candidates = state.attach.search(&req.query).await?;
    complete_route_execution(&state, execution, "/surfaces/search").await?;
    Ok(Json(SearchResponse { candidates }))
}

pub async fn post_track(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TrackRequest>,
) -> Result<Json<TrackResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let execution = authorize_all_surfaces_actions(&state, &headers, &[ActionClass::Manage], Some("track surface")).await?;
    let outcome = state.attach.track(&req.ref_).await?;
    complete_route_execution(&state, execution, "/surfaces/track").await?;
    let info = &outcome.surface;
    Ok(Json(TrackResponse {
        surface_id: info.id.to_string(),
        cg_window_id: info.cg_window_id.expect("tracked surfaces carry pid and cg_window_id"),
        pid: info.pid.expect("tracked surfaces carry pid and cg_window_id"),
        app_name: info.app_name.clone(),
        title: info.title.clone(),
        reused_existing_handle: outcome.reused_existing_handle,
    }))
}
