use axum::{
    Json,
    extract::{Path, State},
};
use porthole_core::surface::SurfaceId;
use porthole_protocol::input::{
    ClickRequest, ClickResponse, KeyRequest, KeyResponse, ScrollRequest, ScrollResponse, TextRequest, TextResponse,
};

use crate::{routes::errors::ApiError, state::AppState};

pub async fn post_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<KeyRequest>,
) -> Result<Json<KeyResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let count = req.events.len();
    state.input.key(&surface_id, &req.events).await?;
    Ok(Json(KeyResponse {
        surface_id: surface_id.to_string(),
        events_sent: count,
    }))
}

pub async fn post_text(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TextRequest>,
) -> Result<Json<TextResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let chars = req.text.chars().count();
    state.input.text(&surface_id, &req.text).await?;
    Ok(Json(TextResponse {
        surface_id: surface_id.to_string(),
        chars_sent: chars,
    }))
}

pub async fn post_click(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ClickRequest>,
) -> Result<Json<ClickResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let spec = (&req).into();
    state.input.click(&surface_id, &spec).await?;
    Ok(Json(ClickResponse {
        surface_id: surface_id.to_string(),
    }))
}

pub async fn post_scroll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ScrollRequest>,
) -> Result<Json<ScrollResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let spec = (&req).into();
    state.input.scroll(&surface_id, &spec).await?;
    Ok(Json(ScrollResponse {
        surface_id: surface_id.to_string(),
    }))
}
