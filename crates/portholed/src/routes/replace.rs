use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use porthole_core::surface::SurfaceId;
use porthole_protocol::launches::{LaunchResponse, ReplaceRequest};

use crate::routes::errors::ApiError;
use crate::routes::launches::{confidence_to_wire, correlation_to_wire, request_to_launch_spec};
use crate::state::AppState;

pub async fn post_replace(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ReplaceRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    // Same auto_dismiss validation as launches.
    if req.auto_dismiss_after_ms == Some(0) {
        return Err(ApiError::from(porthole_core::PortholeError::new(
            porthole_core::ErrorCode::InvalidArgument,
            "auto_dismiss_after_ms must be > 0",
        )));
    }

    let old_id = SurfaceId::from(id);
    let spec = request_to_launch_spec(&req)?;
    let placement = req.placement.as_ref();

    let out = state.replace.replace(&old_id, &spec, placement).await?;

    if let Some(ms) = req.auto_dismiss_after_ms {
        if ms > 0 {
            porthole_core::launch::schedule_auto_dismiss(
                state.adapter.clone(),
                state.handles.clone(),
                out.new.outcome.surface.id.clone(),
                Duration::from_millis(ms),
            );
        }
    }

    let launch_id = format!("launch_{}", uuid::Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: out.new.outcome.surface.id.clone(),
        surface_was_preexisting: out.new.outcome.surface_was_preexisting,
        confidence: confidence_to_wire(out.new.outcome.confidence),
        correlation: correlation_to_wire(out.new.outcome.correlation),
        placement: out.new.placement,
    }))
}
