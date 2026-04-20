use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::adapter::{
    Adapter, Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot,
};
use crate::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use crate::PortholeError;

#[derive(Clone, Default)]
pub struct InMemoryAdapter {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    next_launch_outcome: Option<Result<LaunchOutcome, PortholeError>>,
    next_screenshot: Option<Result<Screenshot, PortholeError>>,
    launch_calls: Vec<ProcessLaunchSpec>,
    screenshot_calls: Vec<SurfaceId>,
}

impl InMemoryAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_next_launch_outcome(&self, outcome: Result<LaunchOutcome, PortholeError>) {
        self.script.lock().await.next_launch_outcome = Some(outcome);
    }

    pub async fn set_next_screenshot(&self, out: Result<Screenshot, PortholeError>) {
        self.script.lock().await.next_screenshot = Some(out);
    }

    pub async fn launch_calls(&self) -> Vec<ProcessLaunchSpec> {
        self.script.lock().await.launch_calls.clone()
    }

    pub async fn screenshot_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.screenshot_calls.clone()
    }

    pub fn make_default_launch_outcome(pid: u32) -> LaunchOutcome {
        let surface = SurfaceInfo {
            id: SurfaceId::new(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title: Some("test window".to_string()),
            app_bundle: Some("com.example.test".to_string()),
            pid: Some(pid),
            parent_surface_id: None,
        };
        LaunchOutcome {
            surface,
            confidence: Confidence::Strong,
            correlation: Correlation::Tag,
            surface_was_preexisting: false,
        }
    }
}

#[async_trait]
impl Adapter for InMemoryAdapter {
    fn name(&self) -> &'static str {
        "in-memory"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut script = self.script.lock().await;
        script.launch_calls.push(spec.clone());
        script
            .next_launch_outcome
            .take()
            .unwrap_or_else(|| Ok(Self::make_default_launch_outcome(4242)))
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        let mut script = self.script.lock().await;
        script.screenshot_calls.push(surface.id.clone());
        script.next_screenshot.take().unwrap_or_else(|| {
            Ok(Screenshot {
                png_bytes: minimal_png(),
                window_bounds_points: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
                content_bounds_points: None,
                scale: 2.0,
                captured_at_unix_ms: 0,
            })
        })
    }
}

fn minimal_png() -> Vec<u8> {
    // 1x1 transparent PNG — smallest valid image. Tests only check presence and shape.
    const BYTES: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49,
        0x44, 0x41, 0x54, 0x78, 0x9c, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    BYTES.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    #[tokio::test]
    async fn launch_records_call_and_returns_default_outcome() {
        let adapter = InMemoryAdapter::new();
        let spec = ProcessLaunchSpec {
            app: "/Applications/Test.app".to_string(),
            args: vec!["--help".to_string()],
            cwd: None,
            env: vec![],
            timeout: std::time::Duration::from_secs(5),
            require_confidence: crate::adapter::RequireConfidence::Strong,
        };
        let outcome = adapter.launch_process(&spec).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        let calls = adapter.launch_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].app, "/Applications/Test.app");
    }

    #[tokio::test]
    async fn screenshot_returns_png_bytes() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let shot = adapter.screenshot(&outcome.surface).await.unwrap();
        assert!(shot.png_bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47]));
    }

    #[tokio::test]
    async fn scripted_error_is_surfaced() {
        let adapter = InMemoryAdapter::new();
        adapter
            .set_next_launch_outcome(Err(PortholeError::new(
                ErrorCode::LaunchCorrelationAmbiguous,
                "two candidates",
            )))
            .await;
        let spec = ProcessLaunchSpec {
            app: "x".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            timeout: std::time::Duration::from_secs(1),
            require_confidence: crate::adapter::RequireConfidence::Strong,
        };
        let err = adapter.launch_process(&spec).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::LaunchCorrelationAmbiguous);
    }
}
