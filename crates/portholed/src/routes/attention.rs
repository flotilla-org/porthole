use axum::{Json, extract::State, http::HeaderMap};
use porthole_core::{agent_policy::ActionClass, attention::AttentionInfo};
use porthole_protocol::attention::DisplaysResponse;

use crate::{
    routes::{
        agent_guard::{authorize_all_surfaces_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn get_attention(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<AttentionInfo>, ApiError> {
    let execution = authorize_all_surfaces_actions(&state, &headers, &[ActionClass::Manage], Some("read attention")).await?;
    let mut info = state.adapter.attention().await?;

    // Resolve focused_surface_id in the daemon, where tracked handles can be
    // matched to the adapter's platform window identity.
    if let Ok(Some(platform_ref)) = state.adapter.focused_platform_surface_ref().await {
        info.focused_surface_id = state.handles.find_by_platform_ref(&platform_ref).await;
    }

    complete_route_execution(&state, execution, "/attention").await?;
    Ok(Json(info))
}

pub async fn get_displays(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<DisplaysResponse>, ApiError> {
    let execution = authorize_all_surfaces_actions(&state, &headers, &[ActionClass::Observe], Some("read displays")).await?;
    let displays = state.adapter.displays().await?;
    complete_route_execution(&state, execution, "/displays").await?;
    Ok(Json(DisplaysResponse { displays }))
}
