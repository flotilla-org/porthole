use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::wait::{WaitRequest, WaitResponse};

use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_wait(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<WaitRequest>,
) -> Result<Json<WaitResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(
        &state,
        &headers,
        surface_id.as_str(),
        &[ActionClass::Observe],
        Some("wait for surface state"),
    )
    .await?;
    let timeout = Duration::from_millis(req.timeout_ms);
    let outcome = state.wait.wait(&surface_id, &req.condition, timeout).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/wait").await?;
    Ok(Json(WaitResponse {
        surface_id: surface_id.to_string(),
        condition: outcome.condition,
        elapsed_ms: outcome.elapsed_ms,
    }))
}
