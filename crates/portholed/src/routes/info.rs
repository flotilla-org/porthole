use axum::extract::State;
use axum::Json;
use porthole_core::permission::PermissionStatus as CorePermission;
use porthole_protocol::info::{AdapterInfo, InfoResponse, PermissionStatus};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Result<Json<InfoResponse>, ApiError> {
    let perms = state.adapter.permissions().await.unwrap_or_default();
    Ok(Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: vec![
                "launch_process".to_string(),
                "screenshot".to_string(),
                "input_key".to_string(),
                "input_text".to_string(),
                "input_click".to_string(),
                "input_scroll".to_string(),
                "wait".to_string(),
                "close".to_string(),
                "focus".to_string(),
                "attention".to_string(),
                "displays".to_string(),
            ],
            permissions: perms.into_iter().map(to_wire_permission).collect(),
        }],
    }))
}

fn to_wire_permission(p: CorePermission) -> PermissionStatus {
    PermissionStatus { name: p.name, granted: p.granted, purpose: p.purpose }
}
