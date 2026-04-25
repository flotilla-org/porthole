use axum::{
    Json,
    extract::{Path, State},
};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use porthole_core::surface::SurfaceId;
use porthole_protocol::screenshot::{Rect, ScreenshotRequest, ScreenshotResponse};

use crate::{routes::errors::ApiError, state::AppState};

pub async fn post_screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(_req): Json<ScreenshotRequest>,
) -> Result<Json<ScreenshotResponse>, ApiError> {
    // session propagation deferred to events/attention plan
    let surface_id = SurfaceId::from(id.clone());
    let info = state.handles.require_alive(&surface_id).await?;
    let shot = state.adapter.screenshot(&info).await?;
    let png_b64 = B64.encode(&shot.png_bytes);
    Ok(Json(ScreenshotResponse {
        surface_id: info.id,
        png_base64: png_b64,
        window_bounds: to_rect(shot.window_bounds_points),
        content_bounds: shot.content_bounds_points.map(to_rect),
        scale: shot.scale,
        captured_at_unix_ms: shot.captured_at_unix_ms,
        session: None,
    }))
}

fn to_rect(r: porthole_core::adapter::Rect) -> Rect {
    Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}
