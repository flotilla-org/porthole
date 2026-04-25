use axum::{Json, extract::State};
use porthole_core::attention::AttentionInfo;
use porthole_protocol::attention::DisplaysResponse;

use crate::{routes::errors::ApiError, state::AppState};

pub async fn get_attention(State(state): State<AppState>) -> Result<Json<AttentionInfo>, ApiError> {
    let mut info = state.adapter.attention().await?;

    // Resolve focused_surface_id: ask the adapter for the frontmost CGWindowID,
    // then look it up in the handle store.
    if let Ok(Some(cg_id)) = state.adapter.frontmost_window_id().await {
        info.focused_surface_id = state.handles.find_by_cg_window_id(cg_id).await;
    }

    Ok(Json(info))
}

pub async fn get_displays(State(state): State<AppState>) -> Result<Json<DisplaysResponse>, ApiError> {
    let displays = state.adapter.displays().await?;
    Ok(Json(DisplaysResponse { displays }))
}
