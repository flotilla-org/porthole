use std::{sync::Arc, time::Duration};

#[cfg(test)]
use crate::adapter::{ArtifactLaunchSpec, Rect};
use crate::{
    ErrorCode, PortholeError,
    adapter::{Adapter, LaunchOutcome, LaunchSpec, ProcessLaunchSpec},
    handle::HandleStore,
    placement::{PlacementOutcome, PlacementSpec, resolve_placement_rect, validate_placement},
    surface::{SurfaceId, SurfaceInfo},
};

pub struct LaunchPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl LaunchPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn launch(&self, spec: &LaunchSpec, placement: Option<&PlacementSpec>) -> Result<LaunchPipelineOutcome, LaunchPipelineError> {
        if spec.require_fresh_surface() && spec.force_place() {
            return Err(LaunchPipelineError::Porthole(PortholeError::new(
                ErrorCode::InvalidArgument,
                "force_place cannot be combined with require_fresh_surface",
            )));
        }

        // 0. Pre-flight: validate user-supplied placement spec.
        if let Some(p) = placement {
            validate_placement(p, &self.adapter).await.map_err(LaunchPipelineError::Porthole)?;
        }

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
            let platform_ref = outcome.surface.platform_ref.clone().ok_or_else(|| {
                LaunchPipelineError::Porthole(PortholeError::new(
                    ErrorCode::InternalError,
                    "preexisting launch surface has no platform_ref",
                ))
            })?;
            let ref_ = crate::search::encode_ref(outcome.surface.pid.unwrap_or(0), platform_ref.clone());
            return Err(LaunchPipelineError::ReturnedExisting(ExistingSurfaceInfo {
                ref_,
                app_name: outcome.surface.app_name.clone(),
                title: outcome.surface.title.clone(),
                pid: outcome.surface.pid.unwrap_or(0),
                platform_ref,
            }));
        }

        // 4. Insert or reuse the handle — prevents duplicate SurfaceIds for the
        //    same platform ref when an attach or prior launch already tracked it.
        let (stored, _reused) = self.handles.track_or_get(outcome.surface.clone()).await;
        let outcome = LaunchOutcome {
            surface: stored,
            ..outcome
        };

        // 5. Resolve + apply placement.
        let placement_outcome = if outcome.surface_was_preexisting {
            if placement.map(|p| !p.is_effectively_empty()).unwrap_or(false) {
                if spec.force_place() {
                    self.apply_placement(&outcome.surface, placement).await
                } else {
                    PlacementOutcome::SkippedPreexisting
                }
            } else {
                PlacementOutcome::NotRequested
            }
        } else {
            self.apply_placement(&outcome.surface, placement).await
        };

        Ok(LaunchPipelineOutcome {
            outcome,
            placement: placement_outcome,
        })
    }

    async fn apply_placement(&self, surface: &SurfaceInfo, placement: Option<&PlacementSpec>) -> PlacementOutcome {
        let Some(p) = placement else {
            return PlacementOutcome::NotRequested;
        };
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
    pub platform_ref: crate::surface::PlatformSurfaceRef,
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
    use crate::{
        adapter::{Confidence, Correlation, RequireConfidence},
        in_memory::InMemoryAdapter,
        placement::{Anchor, DisplayTarget},
        surface::{PlatformSurfaceRef, SurfaceState},
    };

    fn spec(required: RequireConfidence) -> ProcessLaunchSpec {
        ProcessLaunchSpec {
            app: "test".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(1),
            require_confidence: required,
            require_fresh_surface: false,
            force_place: false,
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
            force_place: false,
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
            force_place: false,
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
        outcome.surface.platform_ref = Some(PlatformSurfaceRef::macos(42));
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: true,
            force_place: false,
            timeout: Duration::from_secs(5),
        });
        match pipeline.launch(&spec, None).await {
            Err(LaunchPipelineError::ReturnedExisting(info)) => {
                assert_eq!(info.platform_ref, PlatformSurfaceRef::macos(42));
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
            geometry: Some(Rect {
                x: 10.0,
                y: 20.0,
                w: 800.0,
                h: 600.0,
            }),
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
            geometry: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 500.0,
                h: 500.0,
            }),
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::SkippedPreexisting);
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn force_place_applies_placement_on_preexisting() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);

        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 500.0,
                h: 500.0,
            }),
            anchor: None,
        };
        let mut process = spec_minimal(RequireConfidence::Strong);
        process.force_place = true;
        let spec = LaunchSpec::Process(process);
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::Applied);
        assert_eq!(adapter.place_surface_calls().await.len(), 1);
    }

    #[tokio::test]
    async fn force_place_without_effective_placement_is_not_requested() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_outcome(Ok(outcome)).await;
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);

        let mut process = spec_minimal(RequireConfidence::Strong);
        process.force_place = true;
        let spec = LaunchSpec::Process(process);
        let placement = PlacementSpec::default();
        let result = pipeline.launch(&spec, Some(&placement)).await.unwrap();
        assert_eq!(result.placement, PlacementOutcome::NotRequested);
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn require_fresh_and_force_place_is_invalid() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);

        let mut process = spec_minimal(RequireConfidence::Strong);
        process.require_fresh_surface = true;
        process.force_place = true;
        let spec = LaunchSpec::Process(process);
        match pipeline.launch(&spec, None).await {
            Err(LaunchPipelineError::Porthole(error)) => {
                assert_eq!(error.code, ErrorCode::InvalidArgument);
                assert!(error.message.contains("force_place"));
            }
            other => panic!("expected invalid force_place combination, got {other:?}"),
        }
        assert!(adapter.launch_calls().await.is_empty());
    }

    #[tokio::test]
    async fn placement_failure_reported_as_outcome_not_error() {
        let adapter = Arc::new(InMemoryAdapter::new());
        adapter
            .set_next_place_surface_result(Err(PortholeError::new(ErrorCode::CapabilityMissing, "window refused resize")))
            .await;
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Primary),
            geometry: Some(Rect {
                x: 0.0,
                y: 0.0,
                w: 200.0,
                h: 200.0,
            }),
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

        let _handle = schedule_auto_dismiss(adapter.clone(), handles.clone(), id.clone(), Duration::from_millis(20));
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

    #[tokio::test]
    async fn unknown_display_id_is_invalid_argument_not_soft_failure() {
        use crate::display::DisplayId;

        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let placement = PlacementSpec {
            on_display: Some(DisplayTarget::Id(DisplayId::new("disp_does_not_exist"))),
            geometry: None,
            anchor: None,
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        match pipeline.launch(&spec, Some(&placement)).await {
            Err(LaunchPipelineError::Porthole(e)) => {
                assert_eq!(e.code, ErrorCode::InvalidArgument);
                assert!(e.message.contains("unknown on_display"));
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
        // Critically: adapter.launch_process should NOT have been called.
        assert_eq!(adapter.launch_calls().await.len(), 0);
    }

    #[tokio::test]
    async fn launch_reuses_existing_handle_for_same_platform_ref() {
        use crate::surface::{SurfaceId, SurfaceInfo};

        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        // Seed an existing tracked surface.
        let mut existing = SurfaceInfo::window(SurfaceId::new(), 42);
        existing.platform_ref = Some(PlatformSurfaceRef::macos(777));
        let existing_id = existing.id.clone();
        handles.insert(existing).await;

        // Script a launch outcome whose surface has the same platform ref.
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(42);
        outcome.surface.platform_ref = Some(PlatformSurfaceRef::macos(777));
        outcome.surface_was_preexisting = true;
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        let pipeline = LaunchPipeline::new(adapter, handles);
        let spec = LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: "/tmp/x.pdf".into(),
            require_confidence: RequireConfidence::Plausible,
            require_fresh_surface: false,
            force_place: false,
            timeout: std::time::Duration::from_secs(5),
        });
        let result = pipeline.launch(&spec, None).await.unwrap();
        assert_eq!(
            result.outcome.surface.id, existing_id,
            "launch must return the already-tracked surface id, not a new one"
        );
    }

    #[tokio::test]
    async fn cursor_anchor_centers_at_cursor_position() {
        use crate::{
            attention::{AttentionInfo, CursorPos},
            display::{DisplayId, DisplayInfo, Rect as DRect},
        };

        let adapter = Arc::new(InMemoryAdapter::new());
        // Script displays: one 1920x1080 primary.
        adapter
            .set_next_displays(Ok(vec![DisplayInfo {
                id: DisplayId::new("in-mem-display-0"),
                bounds: DRect {
                    x: 0.0,
                    y: 0.0,
                    w: 1920.0,
                    h: 1080.0,
                },
                scale: 1.0,
                primary: true,
                focused: true,
            }]))
            .await;
        // Script attention with a specific cursor position.
        adapter
            .set_next_attention(Ok(AttentionInfo {
                focused_surface_id: None,
                focused_app_name: None,
                focused_display_id: Some(DisplayId::new("in-mem-display-0")),
                cursor: CursorPos {
                    x: 1000.0,
                    y: 500.0,
                    display_id: Some(DisplayId::new("in-mem-display-0")),
                },
                recently_active_surface_ids: vec![],
            }))
            .await;

        let handles = HandleStore::new();
        let pipeline = LaunchPipeline::new(adapter.clone(), handles);
        let placement = PlacementSpec {
            on_display: None,
            geometry: None,
            anchor: Some(Anchor::Cursor),
        };
        let spec = LaunchSpec::Process(spec_minimal(RequireConfidence::Strong));
        pipeline.launch(&spec, Some(&placement)).await.unwrap();

        let place_calls = adapter.place_surface_calls().await;
        assert_eq!(place_calls.len(), 1, "place_surface should have been called");
        let rect = place_calls[0].1;
        // Default size: 0.7 * 1920 = 1344 (< 1400), 0.7 * 1080 = 756 (< 1000).
        // Centered at cursor (1000, 500): x = 1000 - 672 = 328, y = 500 - 378 = 122.
        // Both within display bounds, so no clamping.
        assert!((rect.x - 328.0).abs() < 2.0, "cursor-anchored x should be ~328, got {}", rect.x);
        assert!((rect.y - 122.0).abs() < 2.0, "cursor-anchored y should be ~122, got {}", rect.y);
        // Verify attention was called exactly once (fetch-once pattern).
        assert_eq!(adapter.attention_calls().await, 1, "attention should be fetched exactly once");
    }
}
