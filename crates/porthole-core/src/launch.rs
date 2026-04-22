use std::sync::Arc;
use std::time::Duration;

use crate::adapter::{Adapter, LaunchOutcome, LaunchSpec, ProcessLaunchSpec, Rect};
#[cfg(test)]
use crate::adapter::ArtifactLaunchSpec;
use crate::handle::HandleStore;
use crate::placement::{Anchor, DisplayTarget, PlacementOutcome, PlacementSpec};
use crate::surface::{SurfaceId, SurfaceInfo};
use crate::{ErrorCode, PortholeError};

pub struct LaunchPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl LaunchPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn launch(
        &self,
        spec: &LaunchSpec,
        placement: Option<&PlacementSpec>,
    ) -> Result<LaunchPipelineOutcome, LaunchPipelineError> {
        // 1. Dispatch to the right adapter method.
        let outcome = match spec {
            LaunchSpec::Process(p) => self.adapter.launch_process(p).await?,
            LaunchSpec::Artifact(a) => self.adapter.launch_artifact(a).await?,
        };

        // 2. Confidence gate.
        if !outcome.confidence.meets(spec.require_confidence()) {
            return Err(LaunchPipelineError::Porthole(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                format!(
                    "launch correlation returned confidence {:?}; required {:?}",
                    outcome.confidence,
                    spec.require_confidence()
                ),
            )));
        }

        // 3. Fresh-surface gate.
        if spec.require_fresh_surface() && outcome.surface_was_preexisting {
            let ref_ = crate::search::encode_ref(
                outcome.surface.pid.unwrap_or(0),
                outcome.surface.cg_window_id.unwrap_or(0),
            );
            return Err(LaunchPipelineError::ReturnedExisting(ExistingSurfaceInfo {
                ref_,
                app_name: outcome.surface.app_name.clone(),
                title: outcome.surface.title.clone(),
                pid: outcome.surface.pid.unwrap_or(0),
                cg_window_id: outcome.surface.cg_window_id.unwrap_or(0),
            }));
        }

        // 4. Insert the handle.
        self.handles.insert(outcome.surface.clone()).await;

        // 5. Resolve + apply placement.
        let placement_outcome = if outcome.surface_was_preexisting {
            if placement.map(|p| !p.is_effectively_empty()).unwrap_or(false) {
                PlacementOutcome::SkippedPreexisting
            } else {
                PlacementOutcome::NotRequested
            }
        } else {
            self.apply_placement(&outcome.surface, placement).await
        };

        Ok(LaunchPipelineOutcome { outcome, placement: placement_outcome })
    }

    async fn apply_placement(
        &self,
        surface: &SurfaceInfo,
        placement: Option<&PlacementSpec>,
    ) -> PlacementOutcome {
        let Some(p) = placement else { return PlacementOutcome::NotRequested; };
        if p.is_effectively_empty() {
            return PlacementOutcome::NotRequested;
        }

        match resolve_placement_rect(p, &self.adapter).await {
            Ok(rect) => match self.adapter.place_surface(surface, rect).await {
                Ok(()) => PlacementOutcome::Applied,
                Err(e) => PlacementOutcome::Failed { reason: e.message },
            },
            Err(reason) => PlacementOutcome::Failed { reason },
        }
    }

    /// Backward-compat: legacy launch_process entry used by routes that
    /// haven't migrated to the unified `launch()` API yet. Calls the new
    /// unified path with no placement.
    pub async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        match self.launch(&LaunchSpec::Process(spec.clone()), None).await {
            Ok(out) => Ok(out.outcome),
            Err(LaunchPipelineError::Porthole(e)) => Err(e),
            Err(LaunchPipelineError::ReturnedExisting(_)) => Err(PortholeError::new(
                ErrorCode::LaunchReturnedExisting,
                "process launch returned existing (should be unreachable for process kind)",
            )),
        }
    }
}

#[derive(Debug)]
pub struct LaunchPipelineOutcome {
    pub outcome: LaunchOutcome,
    pub placement: PlacementOutcome,
}

/// Error body for LaunchReturnedExisting. Callers get everything needed to
/// attach the preexisting surface as a fallback without re-running search.
#[derive(Debug)]
pub struct ExistingSurfaceInfo {
    pub ref_: String, // slice-B opaque ref via porthole_core::search::encode_ref
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub pid: u32,
    pub cg_window_id: u32,
}

#[derive(Debug)]
pub enum LaunchPipelineError {
    Porthole(PortholeError),
    ReturnedExisting(ExistingSurfaceInfo),
}

impl From<PortholeError> for LaunchPipelineError {
    fn from(e: PortholeError) -> Self {
        Self::Porthole(e)
    }
}

/// Resolve a PlacementSpec to a global screen rectangle. Uses the adapter's
/// displays() and attention() to find target display; applies anchor/geometry
/// semantics per spec §5.
async fn resolve_placement_rect(spec: &PlacementSpec, adapter: &Arc<dyn Adapter>) -> Result<Rect, String> {
    let displays = adapter.displays().await.map_err(|e| e.message)?;
    if displays.is_empty() {
        return Err("no displays enumerated".into());
    }

    // 1. Determine target display.
    let target = match &spec.on_display {
        Some(DisplayTarget::Id(id)) => displays
            .iter()
            .find(|d| &d.id == id)
            .cloned()
            .ok_or_else(|| format!("unknown display id '{}'", id.as_str()))?,
        Some(DisplayTarget::Primary) => displays
            .iter()
            .find(|d| d.primary)
            .cloned()
            .unwrap_or_else(|| displays[0].clone()),
        Some(DisplayTarget::Focused) => {
            let attn = adapter.attention().await.map_err(|e| e.message)?;
            match attn.focused_display_id {
                Some(id) => displays
                    .iter()
                    .find(|d| d.id == id)
                    .cloned()
                    .unwrap_or_else(|| displays[0].clone()),
                None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
            }
        }
        None => match spec.anchor {
            Some(Anchor::Cursor) => {
                let attn = adapter.attention().await.map_err(|e| e.message)?;
                displays
                    .iter()
                    .find(|d| {
                        attn.cursor.x >= d.bounds.x
                            && attn.cursor.x < d.bounds.x + d.bounds.w
                            && attn.cursor.y >= d.bounds.y
                            && attn.cursor.y < d.bounds.y + d.bounds.h
                    })
                    .cloned()
                    .unwrap_or_else(|| displays[0].clone())
            }
            Some(Anchor::FocusedDisplay) => {
                let attn = adapter.attention().await.map_err(|e| e.message)?;
                match attn.focused_display_id {
                    Some(id) => displays
                        .iter()
                        .find(|d| d.id == id)
                        .cloned()
                        .unwrap_or_else(|| displays[0].clone()),
                    None => displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone()),
                }
            }
            None => {
                // Geometry supplied without on_display or anchor — applies to primary.
                displays.iter().find(|d| d.primary).cloned().unwrap_or_else(|| displays[0].clone())
            }
        },
    };

    // 2. Compute geometry on that display.
    let global = if let Some(local) = &spec.geometry {
        Rect { x: target.bounds.x + local.x, y: target.bounds.y + local.y, w: local.w, h: local.h }
    } else {
        // No explicit geometry — use a conservative centered default based on display size.
        let w = (target.bounds.w * 0.7).min(1400.0);
        let h = (target.bounds.h * 0.7).min(1000.0);
        let x = target.bounds.x + (target.bounds.w - w) / 2.0;
        let y = target.bounds.y + (target.bounds.h - h) / 2.0;
        Rect { x, y, w, h }
    };

    Ok(global)
}

/// Schedule an auto-dismiss of the surface after `delay`. Returns a JoinHandle
/// that the caller can abort to cancel early. Fire-and-forget is also fine —
/// the timer swallows dead-surface errors.
pub fn schedule_auto_dismiss(
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
    surface_id: SurfaceId,
    delay: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        // Best-effort close. Errors are expected if the surface already died.
        if let Ok(info) = handles.require_alive(&surface_id).await {
            if adapter.close(&info).await.is_ok() {
                let _ = handles.mark_dead(&surface_id).await;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::adapter::{Confidence, Correlation, RequireConfidence};
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceState;

    fn spec(required: RequireConfidence) -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "test".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(1),
            require_confidence: required,
            require_fresh_surface: false,
        }
    }

    fn spec_minimal(rc: RequireConfidence) -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "X".into(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(5),
            require_confidence: rc,
            require_fresh_surface: false,
        }
    }

    #[tokio::test]
    async fn strong_launch_succeeds_and_stores_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles.clone());
        let outcome = pipeline.launch_process(&spec(RequireConfidence::Strong)).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        let stored = handles.get(&outcome.surface.id).await.unwrap();
        assert_eq!(stored.state, SurfaceState::Alive);
    }

    #[tokio::test]
    async fn plausible_adapter_outcome_fails_strong_requirement() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(1);
        outcome.confidence = Confidence::Plausible;
        outcome.correlation = Correlation::PidTree;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let err = pipeline.launch_process(&spec(RequireConfidence::Strong)).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::LaunchCorrelationAmbiguous);
    }

    #[tokio::test]
    async fn plausible_adapter_outcome_succeeds_with_plausible_requirement() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(1);
        outcome.confidence = Confidence::Plausible;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let result = pipeline.launch_process(&spec(RequireConfidence::Plausible)).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn artifact_launch_via_unified_entry_point() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles.clone());
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: false,
            timeout: Duration::from_secs(5),
        });
        let result = pipeline.launch(&spec, None).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::NotRequested);
        assert_eq!(adapter.launch_artifact_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn require_fresh_surface_errors_on_preexisting() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(42);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: true,
            timeout: Duration::from_secs(5),
        });
        match pipeline.launch(&spec, None).await {
            Err(LaunchPipelineError::ReturnedExisting(info)) => {
                assert_eq!(info.cg_window_id, 42);
                assert!(info.ref_.starts_with("ref_"));
            }
            other => panic!("expected ReturnedExisting, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn placement_applied_on_fresh_launch() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 10.0, y: 20.0, w: 800.0, h: 600.0 }),
            anchor: None,
        };
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::Applied);
        let place_calls = adapter.place_surface_calls().await;
        assert_eq!(place_calls.len(), 1);
    }

    #[tokio::test]
    async fn placement_skipped_on_preexisting() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);

        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 0.0, y: 0.0, w: 500.0, h: 500.0 }),
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::SkippedPreexisting);
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn placement_failure_reported_as_outcome_not_error() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_place_surface_result(Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                "window refused resize",
            )))
            .await;
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 }),
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        match result.placement {
            PlacementOutcome::Failed { reason } => assert!(reason.contains("refused")),
            _ => panic!("expected Failed"),
        }
    }

    #[tokio::test]
    async fn auto_dismiss_closes_surface_after_delay() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;

        let _handle =
            schedule_auto_dismiss(adapter.clone(), handles.clone(), id.clone(), Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Expect adapter.close was called once and handle is dead.
        assert_eq!(adapter.close_calls().await.len(), 1);
        let err = handles.require_alive(&id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn auto_dismiss_is_noop_when_surface_already_dead() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        handles.mark_dead(&id).await.unwrap();

        schedule_auto_dismiss(adapter.clone(), handles.clone(), id.clone(), Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(40)).await;

        assert_eq!(adapter.close_calls().await.len(), 0);
    }
}
