use std::collections::BTreeMap;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use porthole_core::adapter::{ArtifactLaunchSpec, LaunchSpec, ProcessLaunchSpec, RequireConfidence};
use porthole_protocol::launches::{
    ArtifactLaunch, LaunchKind, LaunchRequest, LaunchResponse, WireConfidence, WireCorrelation,
};
use uuid::Uuid;

use crate::routes::errors::ApiError;
use crate::state::AppState;

pub async fn post_launches(
    State(state): State<AppState>,
    Json(req): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    // Validate auto_dismiss_after_ms before building spec.
    if req.auto_dismiss_after_ms == Some(0) {
        return Err(ApiError::from(porthole_core::PortholeError::new(
            porthole_core::ErrorCode::InvalidArgument,
            "auto_dismiss_after_ms must be > 0",
        )));
    }

    let spec = request_to_launch_spec(&req)?;
    let placement = req.placement.as_ref();
    let result = state.pipeline.launch(&spec, placement).await?;

    // Schedule auto-dismiss if requested.
    if let Some(ms) = req.auto_dismiss_after_ms {
        if ms > 0 {
            porthole_core::launch::schedule_auto_dismiss(
                state.adapter.clone(),
                state.handles.clone(),
                result.outcome.surface.id.clone(),
                Duration::from_millis(ms),
            );
        }
    }

    let launch_id = format!("launch_{}", Uuid::new_v4().simple());
    Ok(Json(LaunchResponse {
        launch_id,
        surface_id: result.outcome.surface.id.clone(),
        surface_was_preexisting: result.outcome.surface_was_preexisting,
        confidence: confidence_to_wire(result.outcome.confidence),
        correlation: correlation_to_wire(result.outcome.correlation),
        placement: result.placement,
    }))
}

pub(crate) fn request_to_launch_spec(req: &LaunchRequest) -> Result<LaunchSpec, ApiError> {
    let timeout = Duration::from_millis(req.timeout_ms);
    let require_confidence = wire_to_require(req.require_confidence);
    let require_fresh = req.require_fresh_surface;
    match &req.kind {
        LaunchKind::Process(p) => Ok(LaunchSpec::Process(ProcessLaunchSpec {
            app: p.app.clone(),
            args: p.args.clone(),
            cwd: p.cwd.clone(),
            env: to_env_vec(&p.env),
            timeout,
            require_confidence,
            require_fresh_surface: require_fresh,
        })),
        LaunchKind::Artifact(ArtifactLaunch { path }) => {
            if path.starts_with("http://")
                || path.starts_with("https://")
                || path.starts_with("file://")
            {
                return Err(ApiError::from(porthole_core::PortholeError::new(
                    porthole_core::ErrorCode::InvalidArgument,
                    "URL paths are not supported in this slice (defer to browser-CDP)",
                )));
            }
            Ok(LaunchSpec::Artifact(ArtifactLaunchSpec {
                path: std::path::PathBuf::from(path),
                require_confidence,
                require_fresh_surface: require_fresh,
                timeout,
            }))
        }
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

pub(crate) fn confidence_to_wire(c: porthole_core::adapter::Confidence) -> WireConfidence {
    use porthole_core::adapter::Confidence::*;
    match c {
        Strong => WireConfidence::Strong,
        Plausible => WireConfidence::Plausible,
        Weak => WireConfidence::Weak,
    }
}

pub(crate) fn correlation_to_wire(c: porthole_core::adapter::Correlation) -> WireCorrelation {
    use porthole_core::adapter::Correlation::*;
    match c {
        Tag => WireCorrelation::Tag,
        PidTree => WireCorrelation::PidTree,
        Temporal => WireCorrelation::Temporal,
        DocumentMatch => WireCorrelation::DocumentMatch,
        FrontmostChanged => WireCorrelation::FrontmostChanged,
    }
}
