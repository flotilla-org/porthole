use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use porthole_core::{agent_policy::ActionClass, surface::SurfaceId};
use porthole_protocol::{
    agent_permissions::PermissionOperation,
    input::{ClickRequest, ClickResponse, KeyRequest, KeyResponse, ScrollRequest, ScrollResponse, TextRequest, TextResponse},
};

use super::agent_guard::PermissionTrigger;
use crate::{
    routes::{
        agent_guard::{authorize_surface_actions, complete_route_execution},
        errors::ApiError,
    },
    state::AppState,
};

pub async fn post_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<KeyRequest>,
) -> Result<Json<KeyResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(
        &state,
        &headers,
        surface_id.as_str(),
        &[ActionClass::Drive],
        PermissionTrigger::operation(
            "send key events",
            PermissionOperation::Key {
                combinations: req
                    .events
                    .iter()
                    .take(8)
                    .map(|event| {
                        event
                            .modifiers
                            .iter()
                            .map(|modifier| format!("{modifier:?}"))
                            .chain(std::iter::once(event.key.chars().take(80).collect::<String>()))
                            .collect::<Vec<_>>()
                            .join("+")
                    })
                    .collect(),
                event_count: req.events.len(),
            },
        ),
    )
    .await?;
    let count = req.events.len();
    state.input.key(&surface_id, &req.events).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/key").await?;
    Ok(Json(KeyResponse {
        surface_id: surface_id.to_string(),
        events_sent: count,
    }))
}

pub async fn post_text(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<TextRequest>,
) -> Result<Json<TextResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(
        &state,
        &headers,
        surface_id.as_str(),
        &[ActionClass::Drive],
        PermissionTrigger::operation(
            "send text",
            PermissionOperation::Text {
                characters: req.text.chars().count(),
            },
        ),
    )
    .await?;
    let chars = req.text.chars().count();
    state.input.text(&surface_id, &req.text).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/text").await?;
    Ok(Json(TextResponse {
        surface_id: surface_id.to_string(),
        chars_sent: chars,
    }))
}

pub async fn post_click(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ClickRequest>,
) -> Result<Json<ClickResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Drive], Some("click surface")).await?;
    let units = req.units;
    let spec = (&req).into();
    state.input.click(&surface_id, &spec, units).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/click").await?;
    Ok(Json(ClickResponse {
        surface_id: surface_id.to_string(),
    }))
}

pub async fn post_scroll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ScrollRequest>,
) -> Result<Json<ScrollResponse>, ApiError> {
    let surface_id = SurfaceId::from(id);
    let execution = authorize_surface_actions(&state, &headers, surface_id.as_str(), &[ActionClass::Drive], Some("scroll surface")).await?;
    let units = req.units;
    let spec = (&req).into();
    state.input.scroll(&surface_id, &spec, units).await?;
    complete_route_execution(&state, execution, "/surfaces/{id}/scroll").await?;
    Ok(Json(ScrollResponse {
        surface_id: surface_id.to_string(),
    }))
}
