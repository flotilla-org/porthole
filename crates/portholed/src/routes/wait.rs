use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::wait::{WaitRequest, WaitResponse};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_wait(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<WaitRequest>,
) -> Result<Json<WaitResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let timeout = Duration::from_millis(req.timeout_ms);
    let outcome = state.wait.wait(&surface_id, &req.condition, timeout).await?;
    Ok(Json(WaitResponse {
        surface_id: surface_id.to_string(),
        condition: outcome.condition,
        elapsed_ms: outcome.elapsed_ms,
    }))
}
