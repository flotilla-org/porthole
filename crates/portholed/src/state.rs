use std::sync::Arc;
use std::time::Instant;

use porthole_core::adapter::Adapter;
use porthole_core::handle::HandleStore;
use porthole_core::launch::LaunchPipeline;

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            handles,
            pipeline,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
