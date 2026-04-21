use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::close_focus::{CloseRequest, CloseResponse, FocusRequest, FocusResponse};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_close(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<CloseRequest>,
) -> Result<Json<CloseResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    state.input.close(&surface_id).await?;
    Ok(Json(CloseResponse { surface_id: surface_id.to_string(), closed: true }))
}

pub async fn post_focus(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<FocusRequest>,
) -> Result<Json<FocusResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    state.input.focus(&surface_id).await?;
    Ok(Json(FocusResponse { surface_id: surface_id.to_string(), focused: true }))
}
