use std::{sync::Arc, time::Instant};

use porthole_core::{
    adapter::Adapter, attach_pipeline::AttachPipeline, handle::HandleStore, input_pipeline::InputPipeline, launch::LaunchPipeline,
    replace_pipeline::ReplacePipeline, wait_pipeline::WaitPipeline,
};

use crate::capture_registry::CaptureRegistry;

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub replace: Arc<ReplacePipeline>,
    pub input: Arc<InputPipeline>,
    pub wait: Arc<WaitPipeline>,
    pub attach: Arc<AttachPipeline>,
    pub capture: CaptureRegistry,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        Self::new_with_capture_registry(adapter, CaptureRegistry::disabled())
    }

    pub fn new_with_capture_socket(adapter: Arc<dyn Adapter>, fd_socket_path: std::path::PathBuf) -> std::io::Result<Self> {
        Ok(Self::new_with_capture_registry(
            adapter,
            CaptureRegistry::with_fd_socket(fd_socket_path)?,
        ))
    }

    pub fn new_with_capture_registry(adapter: Arc<dyn Adapter>, capture: CaptureRegistry) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = Arc::new(ReplacePipeline::new(adapter.clone(), handles.clone(), pipeline.clone()));
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles.clone()));
        let wait = Arc::new(WaitPipeline::new(adapter.clone(), handles.clone()));
        let attach = Arc::new(AttachPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            handles,
            pipeline,
            replace,
            input,
            wait,
            attach,
            capture,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
