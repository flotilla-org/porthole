use axum::extract::State;
use axum::Json;
use porthole_protocol::info::{AdapterInfo, InfoResponse};

use crate::state::AppState;

pub async fn get_info(State(state): State<AppState>) -> Json<InfoResponse> {
    Json(InfoResponse {
        daemon_version: state.daemon_version.to_string(),
        uptime_seconds: state.uptime_seconds(),
        adapters: vec![AdapterInfo {
            name: state.adapter.name().to_string(),
            loaded: true,
            capabilities: vec!["launch_process".to_string(), "screenshot".to_string()],
        }],
    })
}
