use axum::extract::State;
use axum::Json;
use porthole_protocol::search::{
    SearchRequest, SearchResponse, TrackRequest, TrackResponse,
};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_search(
    State(state): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let candidates = state.attach.search(&req.query).await?;
    Ok(Json(SearchResponse { candidates }))
}

pub async fn post_track(
    State(state): State<AppState>,
    Json(req): Json<TrackRequest>,
) -> Result<Json<TrackResponse>, ApiError> {
    // session intentionally dropped until SSE events slice
    let outcome = state.attach.track(&req.ref_).await?;
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
