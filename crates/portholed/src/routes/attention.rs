use axum::extract::State;
use axum::Json;
use porthole_core::attention::AttentionInfo;
use porthole_protocol::attention::DisplaysResponse;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_attention(State(state): State<AppState>) -> Result<Json<AttentionInfo>, ApiError> {
    let info = state.adapter.attention().await?;
    Ok(Json(info))
}

pub async fn get_displays(State(state): State<AppState>) -> Result<Json<DisplaysResponse>, ApiError> {
    let displays = state.adapter.displays().await?;
    Ok(Json(DisplaysResponse { displays }))
}
