use std::sync::Arc;

use crate::{
    PortholeError,
    adapter::Adapter,
    handle::HandleStore,
    launch::{ExistingSurfaceInfo, LaunchPipeline, LaunchPipelineError, LaunchPipelineOutcome},
    placement::{DisplayTarget, PlacementSpec},
    surface::SurfaceId,
};

pub struct ReplacePipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
    launch: Arc<LaunchPipeline>,
}

#[derive(Debug)]
pub struct ReplaceOutcome {
    pub new: LaunchPipelineOutcome,
    pub old_surface_id: SurfaceId,
}

#[derive(Debug)]
pub enum ReplacePipelineError {
    Porthole { error: PortholeError, old_handle_alive: bool },
    ReturnedExisting { info: ExistingSurfaceInfo, old_handle_alive: bool },
}

impl ReplacePipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore, launch: Arc<LaunchPipeline>) -> Self {
        Self { adapter, handles, launch }
    }

    pub async fn replace(
        &self,
        old_id: &SurfaceId,
        new_spec: &crate::adapter::LaunchSpec,
        caller_placement: Option<&PlacementSpec>,
    ) -> Result<ReplaceOutcome, ReplacePipelineError> {
        // 1. Snapshot (best-effort — snapshot failure doesn't abort).
        let old_info = match self.handles.require_alive(old_id).await {
            Ok(info) => info,
            Err(e) => {
                // Surface is already dead or missing — old handle is NOT alive.
                return Err(ReplacePipelineError::Porthole {
                    error: e,
                    old_handle_alive: false,
                });
            }
        };
        let snapshot = self.adapter.snapshot_geometry(&old_info).await.ok();

        // 2. Close old.
        if let Err(close_err) = self.adapter.close(&old_info).await {
            // old_handle_alive: false iff the adapter told us the surface is already dead.
            // Any other close error (SystemPermissionNeeded, CloseFailed, etc.) means the surface
            // is likely still there.
            let old_handle_alive = close_err.code != crate::ErrorCode::SurfaceDead;
            // Keep the HandleStore in sync with reality: if the adapter
            // reports SurfaceDead, the surface is genuinely gone at the OS
            // level and the handle must transition to Dead. Other error
            // codes leave the surface alive (possibly a save dialog, AX
            // permission issue, etc.) and the handle stays tracked.
            if !old_handle_alive {
                let _ = self.handles.mark_dead(old_id).await;
            }
            return Err(ReplacePipelineError::Porthole {
                error: close_err,
                old_handle_alive,
            });
        }
        self.handles.mark_dead(old_id).await.map_err(|e| ReplacePipelineError::Porthole {
            error: e,
            old_handle_alive: false,
        })?;

        // 3. Inheritance: inject snapshot only if caller_placement is None AND we have a snapshot.
        let inherited = match (caller_placement, snapshot) {
            (None, Some(snap)) => Some(PlacementSpec {
                on_display: Some(DisplayTarget::Id(snap.display_id)),
                geometry: Some(snap.display_local),
                anchor: None,
            }),
            _ => None,
        };
        let effective = inherited.as_ref().or(caller_placement);

        // 4. Launch the replacement.
        match self.launch.launch(new_spec, effective).await {
            Ok(out) => Ok(ReplaceOutcome {
                new: out,
                old_surface_id: old_id.clone(),
            }),
            Err(LaunchPipelineError::Porthole(e)) => Err(ReplacePipelineError::Porthole {
                error: e,
                old_handle_alive: false,
            }),
            Err(LaunchPipelineError::ReturnedExisting(info)) => {
                Err(ReplacePipelineError::ReturnedExisting {
                    info,
                    old_handle_alive: false, // old was already closed by step 2
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ErrorCode,
        adapter::{ArtifactLaunchSpec, LaunchSpec, RequireConfidence},
        display::{DisplayId, Rect},
        in_memory::InMemoryAdapter,
        placement::{GeometrySnapshot, PlacementOutcome},
        surface::SurfaceInfo,
    };

    async fn tracked_surface(handles: &HandleStore, pid: u32, cg: u32) -> SurfaceId {
        let mut info = SurfaceInfo::window(SurfaceId::new(), pid);
        info.cg_window_id = Some(cg);
        let id = info.id.clone();
        handles.insert(info).await;
        id
    }

    fn artifact_spec(path: &str, fresh: bool) -> LaunchSpec {
        LaunchSpec::Artifact(ArtifactLaunchSpec {
            path: path.into(),
            require_confidence: RequireConfidence::Strong,
            require_fresh_surface: fresh,
            timeout: std::time::Duration::from_secs(5),
        })
    }

    #[tokio::test]
    async fn replace_inherits_snapshot_when_placement_absent() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        // Use the in-memory adapter's default display id so resolve_placement_rect can find it.
        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect {
                    x: 100.0,
                    y: 50.0,
                    w: 800.0,
                    h: 600.0,
                },
            }))
            .await;

        let out = replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), None).await.unwrap();
        assert_eq!(out.old_surface_id, old_id);
        assert_eq!(out.new.placement, PlacementOutcome::Applied);
        // Old handle is dead now.
        let err = handles.require_alive(&old_id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn replace_with_empty_placement_does_not_inherit() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_snapshot_geometry(Ok(GeometrySnapshot {
                display_id: DisplayId::new("in-mem-display-0"),
                display_local: Rect {
                    x: 100.0,
                    y: 50.0,
                    w: 800.0,
                    h: 600.0,
                },
            }))
            .await;

        // Caller passes Some(PlacementSpec::default()) — empty but present.
        let empty = PlacementSpec::default();
        let out = replace
            .replace(&old_id, &artifact_spec("/tmp/new.pdf", false), Some(&empty))
            .await
            .unwrap();
        assert_eq!(out.new.placement, PlacementOutcome::NotRequested);
        // place_surface should NOT have been called since placement was effectively empty.
        assert!(adapter.place_surface_calls().await.is_empty());
    }

    #[tokio::test]
    async fn replace_close_failure_keeps_old_handle_alive() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_close_result(Err(PortholeError::new(ErrorCode::CloseFailed, "save dialog blocking close")))
            .await;

        match replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), None).await {
            Err(ReplacePipelineError::Porthole { error, old_handle_alive }) => {
                assert_eq!(error.code, ErrorCode::CloseFailed);
                assert!(old_handle_alive);
                // Old handle still alive.
                assert!(handles.require_alive(&old_id).await.is_ok());
            }
            other => panic!("expected Porthole(CloseFailed), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replace_close_surface_dead_reports_old_handle_not_alive() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_close_result(Err(PortholeError::new(ErrorCode::SurfaceDead, "surface already gone")))
            .await;

        match replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", false), None).await {
            Err(ReplacePipelineError::Porthole { error, old_handle_alive }) => {
                assert_eq!(error.code, ErrorCode::SurfaceDead);
                assert!(!old_handle_alive, "surface_dead means old handle is not alive");
            }
            other => panic!("expected Porthole(SurfaceDead), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replace_close_surface_dead_marks_old_handle_dead_in_store() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;
        adapter
            .set_next_close_result(Err(PortholeError::new(ErrorCode::SurfaceDead, "window already closed by user")))
            .await;

        let result = replace.replace(&old_id, &artifact_spec("/tmp/x.pdf", false), None).await;
        match result {
            Err(ReplacePipelineError::Porthole { error, old_handle_alive }) => {
                assert_eq!(error.code, ErrorCode::SurfaceDead);
                assert!(!old_handle_alive);
            }
            other => panic!("expected Porthole error, got {other:?}"),
        }

        // Handle must now be Dead in the store — require_alive should fail with SurfaceDead.
        let err = handles.require_alive(&old_id).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::SurfaceDead);
    }

    #[tokio::test]
    async fn replace_returned_existing_kills_old_handle() {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let launch = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = ReplacePipeline::new(adapter.clone(), handles.clone(), launch);

        let old_id = tracked_surface(&handles, 100, 50).await;

        // Script a fresh_surface violation on the replacement launch.
        let mut outcome = InMemoryAdapter::make_default_launch_outcome(77);
        outcome.surface_was_preexisting = true;
        outcome.surface.cg_window_id = Some(99);
        adapter.set_next_launch_artifact_outcome(Ok(outcome)).await;

        match replace.replace(&old_id, &artifact_spec("/tmp/new.pdf", true), None).await {
            Err(ReplacePipelineError::ReturnedExisting { old_handle_alive, .. }) => {
                assert!(!old_handle_alive, "old should have been closed in step 2");
                let err = handles.require_alive(&old_id).await.unwrap_err();
                assert_eq!(err.code, ErrorCode::SurfaceDead);
            }
            other => panic!("expected ReturnedExisting, got {other:?}"),
        }
    }
}
