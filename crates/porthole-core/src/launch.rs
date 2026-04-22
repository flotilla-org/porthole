use std::sync::Arc;

use crate::adapter::{Adapter, LaunchOutcome, ProcessLaunchSpec};
use crate::handle::HandleStore;
use crate::{ErrorCode, PortholeError};

pub struct LaunchPipeline {
    adapter: Arc<dyn Adapter>,
    handles: HandleStore,
}

impl LaunchPipeline {
    pub fn new(adapter: Arc<dyn Adapter>, handles: HandleStore) -> Self {
        Self { adapter, handles }
    }

    pub async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let outcome = self.adapter.launch_process(spec).await?;
        if !outcome.confidence.meets(spec.require_confidence) {
            return Err(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                format!(
                    "launch correlation returned confidence {:?}; required {:?}",
                    outcome.confidence, spec.require_confidence
                ),
            ));
        }
        self.handles.insert(outcome.surface.clone()).await;
        Ok(outcome)
    }
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
}
