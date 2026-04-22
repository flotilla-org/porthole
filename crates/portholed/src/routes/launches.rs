use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use porthole_core::adapter::{ProcessLaunchSpec, RequireConfidence};
use porthole_protocol::launches::{
    LaunchKind, LaunchRequest, LaunchResponse, WireConfidence, WireCorrelation,
};
use uuid::Uuid;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_launches(
    State(state): State<AppState>,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    let spec = request_to_spec(&req)?;
    let outcome = state.pipeline.launch_process(&spec).await?;
    let launch_id = format!("launch_{}", Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: outcome.surface.id.clone(),
        surface_was_preexisting: outcome.surface_was_preexisting,
        confidence: confidence_to_wire(outcome.confidence),
        correlation: correlation_to_wire(outcome.correlation),
    }))
}

fn request_to_spec(req: &LaunchRequest) -> Result<ProcessLaunchSpec, ApiError> {
    // req.session is intentionally dropped here; propagation deferred to events/attention plan
    match &req.kind {
        LaunchKind::Process(p) => Ok(ProcessLaunchSpec {
            app: p.app.clone(),
            args: p.args.clone(),
            cwd: p.cwd.clone(),
            env: to_env_vec(&p.env),
            timeout: Duration::from_millis(req.timeout_ms),
            require_confidence: wire_to_require(req.require_confidence),
            require_fresh_surface: false,
        }),
    }
}

fn to_env_vec(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn wire_to_require(c: WireConfidence) -> RequireConfidence {
    match c {
        WireConfidence::Strong => RequireConfidence::Strong,
        WireConfidence::Plausible => RequireConfidence::Plausible,
        WireConfidence::Weak => RequireConfidence::Weak,
    }
}

fn confidence_to_wire(c: porthole_core::adapter::Confidence) -> WireConfidence {
    use porthole_core::adapter::Confidence::*;
    match c {
        Strong => WireConfidence::Strong,
        Plausible => WireConfidence::Plausible,
        Weak => WireConfidence::Weak,
    }
}

fn correlation_to_wire(c: porthole_core::adapter::Correlation) -> WireCorrelation {
    use porthole_core::adapter::Correlation::*;
    match c {
        Tag => WireCorrelation::Tag,
        PidTree => WireCorrelation::PidTree,
        Temporal => WireCorrelation::Temporal,
        DocumentMatch => WireCorrelation::DocumentMatch,
        FrontmostChanged => WireCorrelation::FrontmostChanged,
    }
}
