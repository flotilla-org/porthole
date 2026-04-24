use std::sync::Arc;
use std::time::Duration;

use regex::Regex;

use crate::adapter::Adapter;
use crate::handle::HandleStore;
use crate::surface::SurfaceId;
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
use crate::{ErrorCode, PortholeError};

// Alias for clarity in preflight match arms.
use WaitCondition as Wc;

pub struct WaitPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

#[derive(Debug)]
pub struct WaitTimeoutInfo {
    pub last_observed: LastObserved,
    pub elapsed_ms: u64,
}

impl WaitPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn wait(
        &self,
        surface: &SurfaceId,
        condition: &WaitCondition,
        timeout_duration: Duration,
    ) -> Result<WaitOutcome, WaitPipelineError> {
        validate_condition(condition)?;
        let info = self.handles.require_alive(surface).await.map_err(WaitPipelineError::Porthole)?;

        // Preflight: check permissions before dispatching into the adapter.
        // Stable/Dirty conditions use frame-diff (screen_recording + accessibility).
        // All other conditions need accessibility only.
        let required: &[&str] = match condition {
            Wc::Stable { .. } | Wc::Dirty { .. } | Wc::TitleMatches { .. } => &["screen_recording", "accessibility"],
            _ => &["accessibility"],
        };
        for name in required {
            self.adapter
                .ensure_system_permission(name)
                .await
                .map_err(WaitPipelineError::Porthole)?;
        }

        let deadline = std::time::Instant::now() + timeout_duration;
        match self.adapter.wait(&info, condition, deadline).await {
            Ok(outcome) => Ok(outcome),
            Err(wait_timeout) => Err(WaitPipelineError::Timeout(WaitTimeoutInfo {
                last_observed: wait_timeout.last_observed,
                elapsed_ms: wait_timeout.elapsed_ms,
            })),
        }
    }
}

#[derive(Debug)]
pub enum WaitPipelineError {
    Porthole(PortholeError),
    Timeout(WaitTimeoutInfo),
}

fn validate_condition(condition: &WaitCondition) -> Result<(), WaitPipelineError> {
    match condition {
        WaitCondition::Stable { threshold_pct, .. } | WaitCondition::Dirty { threshold_pct } => {
            if !threshold_pct.is_finite() || *threshold_pct < 0.0 || *threshold_pct > 100.0 {
                return Err(WaitPipelineError::Porthole(PortholeError::new(
                    ErrorCode::InvalidArgument,
                    format!("threshold_pct must be in [0, 100]; got {threshold_pct}"),
                )));
            }
            Ok(())
        }
        WaitCondition::TitleMatches { pattern } => {
            Regex::new(pattern).map_err(|e| {
                WaitPipelineError::Porthole(PortholeError::new(
                    ErrorCode::InvalidArgument,
                    format!("invalid regex '{pattern}': {e}"),
                ))
            })?;
            Ok(())
        }
        WaitCondition::Exists | WaitCondition::Gone => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemoryAdapter;
    use crate::surface::SurfaceInfo;

    async fn setup() -> (Arc<InMemoryAdapter>, HandleStore, SurfaceId) {
        let adapter = Arc::new(InMemoryAdapter::new());
        let handles = HandleStore::new();
        let info = SurfaceInfo::window(SurfaceId::new(), 1);
        let id = info.id.clone();
        handles.insert(info).await;
        (adapter, handles, id)
    }

    #[tokio::test]
    async fn exists_condition_returns_quickly() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let outcome = pipeline.wait(&id, &WaitCondition::Exists, Duration::from_secs(5)).await.unwrap();
        assert_eq!(outcome.condition, "exists");
    }

    #[tokio::test]
    async fn invalid_regex_errors_as_invalid_argument() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let err = pipeline
            .wait(
                &id,
                &WaitCondition::TitleMatches { pattern: "[invalid".to_string() },
                Duration::from_secs(1),
            )
            .await;
        match err {
            Err(WaitPipelineError::Porthole(e)) => assert_eq!(e.code, ErrorCode::InvalidArgument),
            _ => panic!("expected invalid_argument error"),
        }
    }

    #[tokio::test]
    async fn timeout_surfaces_last_observed() {
        let (adapter, handles, id) = setup().await;
        use crate::wait::{LastObserved, WaitTimeout};
        adapter
            .set_next_wait_result(Err(WaitTimeout {
                last_observed: LastObserved::FrameChange {
                    last_change_ms_ago: 500,
                    last_change_pct: 0.3,
                },
                elapsed_ms: 100,
            }))
            .await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let err = pipeline
            .wait(&id, &WaitCondition::Stable { window_ms: 1500, threshold_pct: 1.0 }, Duration::from_secs(5))
            .await
            .unwrap_err();
        match err {
            WaitPipelineError::Timeout(info) => {
                assert_eq!(info.elapsed_ms, 100);
                match info.last_observed {
                    LastObserved::FrameChange { last_change_ms_ago, last_change_pct } => {
                        assert_eq!(last_change_ms_ago, 500);
                        assert!((last_change_pct - 0.3).abs() < 1e-9);
                    }
                    other => panic!("expected FrameChange, got {other:?}"),
                }
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_threshold_pct_rejected() {
        let (adapter, handles, id) = setup().await;
        let pipeline = WaitPipeline::new(adapter, handles);
        let err = pipeline
            .wait(
                &id,
                &WaitCondition::Dirty { threshold_pct: -1.0 },
                Duration::from_secs(1),
            )
            .await;
        match err {
            Err(WaitPipelineError::Porthole(e)) => assert_eq!(e.code, ErrorCode::InvalidArgument),
            _ => panic!("expected invalid_argument error"),
        }
    }
}
