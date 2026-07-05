use std::{sync::Arc, time::Instant};

#[cfg(target_os = "linux")]
use porthole_adapter_kwin::KWinAdapter;
use porthole_core::{
    adapter::Adapter, attach_pipeline::AttachPipeline, handle::HandleStore, input_pipeline::InputPipeline, launch::LaunchPipeline,
    replace_pipeline::ReplacePipeline, wait_pipeline::WaitPipeline,
};

use crate::{agent_store::AgentPolicyStore, capture_registry::CaptureRegistry, events::EventBus};

#[derive(Clone)]
pub struct AppState {
    pub adapter: Arc<dyn Adapter>,
    #[cfg(target_os = "linux")]
    pub kwin_adapter: Option<Arc<KWinAdapter>>,
    pub handles: HandleStore,
    pub pipeline: Arc<LaunchPipeline>,
    pub replace: Arc<ReplacePipeline>,
    pub input: Arc<InputPipeline>,
    pub wait: Arc<WaitPipeline>,
    pub attach: Arc<AttachPipeline>,
    pub capture: CaptureRegistry,
    pub agent_store: AgentPolicyStore,
    pub events: EventBus,
    pub started_at: Instant,
    pub daemon_version: &'static str,
}

impl AppState {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        Self::new_with_capture_registry(adapter, CaptureRegistry::disabled())
    }

    pub fn new_with_capture_socket(adapter: Arc<dyn Adapter>, fd_socket_path: std::path::PathBuf) -> std::io::Result<Self> {
        // Test/server helper: production wiring uses serve_with_agent_policy so
        // HTTP authorization and capture fd authorization share the real store.
        let agent_store = AgentPolicyStore::open_in_memory_sync().expect("in-memory agent policy store should initialize");
        let capture = CaptureRegistry::with_fd_socket_and_agent_policy(fd_socket_path, agent_store.clone())?;
        Ok(Self::new_with_agent_policy_and_capture(
            adapter,
            capture,
            agent_store,
            EventBus::new(),
        ))
    }

    pub fn new_with_capture_registry(adapter: Arc<dyn Adapter>, capture: CaptureRegistry) -> Self {
        Self::new_with_agent_policy_and_capture(
            adapter,
            capture,
            AgentPolicyStore::open_in_memory_sync().expect("in-memory agent policy store should initialize"),
            EventBus::new(),
        )
    }

    pub fn new_with_agent_policy(adapter: Arc<dyn Adapter>, agent_store: AgentPolicyStore, events: EventBus) -> Self {
        Self::new_with_agent_policy_and_capture(adapter, CaptureRegistry::disabled(), agent_store, events)
    }

    pub fn new_with_agent_policy_and_capture(
        adapter: Arc<dyn Adapter>,
        capture: CaptureRegistry,
        agent_store: AgentPolicyStore,
        events: EventBus,
    ) -> Self {
        let handles = HandleStore::new();
        let pipeline = Arc::new(LaunchPipeline::new(adapter.clone(), handles.clone()));
        let replace = Arc::new(ReplacePipeline::new(adapter.clone(), handles.clone(), pipeline.clone()));
        let input = Arc::new(InputPipeline::new(adapter.clone(), handles.clone()));
        let wait = Arc::new(WaitPipeline::new(adapter.clone(), handles.clone()));
        let attach = Arc::new(AttachPipeline::new(adapter.clone(), handles.clone()));
        Self {
            adapter,
            #[cfg(target_os = "linux")]
            kwin_adapter: None,
            handles,
            pipeline,
            replace,
            input,
            wait,
            attach,
            capture,
            agent_store,
            events,
            started_at: Instant::now(),
            daemon_version: env!("CARGO_PKG_VERSION"),
        }
    }

    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn with_kwin_adapter(mut self, kwin_adapter: Arc<KWinAdapter>) -> Self {
        self.kwin_adapter = Some(kwin_adapter);
        self
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started_at.elapsed().as_secs()
    }
}
