use axum::extract::State;
use axum::Json;
use porthole_core::permission::SystemPermissionStatus as CoreSystemPermission;
use porthole_protocol::info::{AdapterInfo, InfoResponse, SystemPermissionStatus};

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Result<Json<InfoResponse>, ApiError> {
    let perms = state.adapter.system_permissions().await.unwrap_or_default();
    Ok(Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: state.adapter.capabilities().into_iter().map(String::from).collect(),
            system_permissions: perms.into_iter().map(to_wire_permission).collect(),
        }],
    }))
}

fn to_wire_permission(p: CoreSystemPermission) -> SystemPermissionStatus {
    SystemPermissionStatus { name: p.name, granted: p.granted, purpose: p.purpose }
}
