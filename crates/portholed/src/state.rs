use std::sync::Arc;
use std::time::Instant;

use porthole_core::adapter::Adapter;
use porthole_core::handle::HandleStore;
use porthole_core::input_pipeline::InputPipeline;
use porthole_core::launch::LaunchPipeline;
use porthole_core::wait_pipeline::WaitPipeline;

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub input: Arc<InputPipeline>,
    pub wait: Arc<WaitPipeline>,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles.clone()));
        let wait = Arc::new(WaitPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            handles,
            pipeline,
            input,
            wait,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
