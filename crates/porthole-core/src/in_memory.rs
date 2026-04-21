use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::adapter::{
    Adapter, Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot,
};
use crate::attention::{AttentionInfo, CursorPos};
use crate::display::{DisplayId, DisplayInfo, Rect as DisplayRect};
use crate::input::{ClickSpec, KeyEvent, ScrollSpec};
use crate::permission::PermissionStatus;
use crate::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use crate::wait::{LastObserved, WaitCondition, WaitOutcome};
use crate::PortholeError;

#[derive(Clone, Default)]
pub struct InMemoryAdapter {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    next_launch_outcome: Option<Result<LaunchOutcome, PortholeError>>,
    next_screenshot: Option<Result<Screenshot, PortholeError>>,
    next_key_result: Option<Result<(), PortholeError>>,
    next_text_result: Option<Result<(), PortholeError>>,
    next_click_result: Option<Result<(), PortholeError>>,
    next_scroll_result: Option<Result<(), PortholeError>>,
    next_close_result: Option<Result<(), PortholeError>>,
    next_focus_result: Option<Result<(), PortholeError>>,
    next_wait_result: Option<Result<WaitOutcome, PortholeError>>,
    next_wait_last_observed: Option<Result<LastObserved, PortholeError>>,
    next_attention: Option<Result<AttentionInfo, PortholeError>>,
    next_displays: Option<Result<Vec<DisplayInfo>, PortholeError>>,
    next_permissions: Option<Result<Vec<PermissionStatus>, PortholeError>>,

    launch_calls: Vec<ProcessLaunchSpec>,
    screenshot_calls: Vec<SurfaceId>,
    key_calls: Vec<(SurfaceId, Vec<KeyEvent>)>,
    text_calls: Vec<(SurfaceId, String)>,
    click_calls: Vec<(SurfaceId, ClickSpec)>,
    scroll_calls: Vec<(SurfaceId, ScrollSpec)>,
    close_calls: Vec<SurfaceId>,
    focus_calls: Vec<SurfaceId>,
    wait_calls: Vec<(SurfaceId, WaitCondition)>,
    attention_calls: usize,
    displays_calls: usize,
    permissions_calls: usize,
}

impl InMemoryAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    // Scripting setters — existing two retained:
    pub async fn set_next_launch_outcome(&self, v: Result<LaunchOutcome, PortholeError>) {
        self.script.lock().await.next_launch_outcome = Some(v);
    }
    pub async fn set_next_screenshot(&self, v: Result<Screenshot, PortholeError>) {
        self.script.lock().await.next_screenshot = Some(v);
    }

    // New scripting setters:
    pub async fn set_next_key_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_key_result = Some(v);
    }
    pub async fn set_next_text_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_text_result = Some(v);
    }
    pub async fn set_next_click_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_click_result = Some(v);
    }
    pub async fn set_next_scroll_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_scroll_result = Some(v);
    }
    pub async fn set_next_close_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_close_result = Some(v);
    }
    pub async fn set_next_focus_result(&self, v: Result<(), PortholeError>) {
        self.script.lock().await.next_focus_result = Some(v);
    }
    pub async fn set_next_wait_result(&self, v: Result<WaitOutcome, PortholeError>) {
        self.script.lock().await.next_wait_result = Some(v);
    }
    pub async fn set_next_wait_last_observed(&self, v: Result<LastObserved, PortholeError>) {
        self.script.lock().await.next_wait_last_observed = Some(v);
    }
    pub async fn set_next_attention(&self, v: Result<AttentionInfo, PortholeError>) {
        self.script.lock().await.next_attention = Some(v);
    }
    pub async fn set_next_displays(&self, v: Result<Vec<DisplayInfo>, PortholeError>) {
        self.script.lock().await.next_displays = Some(v);
    }
    pub async fn set_next_permissions(&self, v: Result<Vec<PermissionStatus>, PortholeError>) {
        self.script.lock().await.next_permissions = Some(v);
    }

    // Recorders:
    pub async fn launch_calls(&self) -> Vec<ProcessLaunchSpec> {
        self.script.lock().await.launch_calls.clone()
    }
    pub async fn screenshot_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.screenshot_calls.clone()
    }
    pub async fn key_calls(&self) -> Vec<(SurfaceId, Vec<KeyEvent>)> {
        self.script.lock().await.key_calls.clone()
    }
    pub async fn text_calls(&self) -> Vec<(SurfaceId, String)> {
        self.script.lock().await.text_calls.clone()
    }
    pub async fn click_calls(&self) -> Vec<(SurfaceId, ClickSpec)> {
        self.script.lock().await.click_calls.clone()
    }
    pub async fn scroll_calls(&self) -> Vec<(SurfaceId, ScrollSpec)> {
        self.script.lock().await.scroll_calls.clone()
    }
    pub async fn close_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.close_calls.clone()
    }
    pub async fn focus_calls(&self) -> Vec<SurfaceId> {
        self.script.lock().await.focus_calls.clone()
    }
    pub async fn wait_calls(&self) -> Vec<(SurfaceId, WaitCondition)> {
        self.script.lock().await.wait_calls.clone()
    }
    pub async fn attention_calls(&self) -> usize {
        self.script.lock().await.attention_calls
    }
    pub async fn displays_calls(&self) -> usize {
        self.script.lock().await.displays_calls
    }
    pub async fn permissions_calls(&self) -> usize {
        self.script.lock().await.permissions_calls
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

    pub fn default_attention() -> AttentionInfo {
        AttentionInfo {
            focused_surface_id: None,
            focused_app_bundle: None,
            focused_display_id: None,
            cursor: CursorPos { x: 0.0, y: 0.0, display_id_index: None },
            recently_active_surface_ids: vec![],
        }
    }

    pub fn default_displays() -> Vec<DisplayInfo> {
        vec![DisplayInfo {
            id: DisplayId::new("in-mem-display-0"),
            bounds: DisplayRect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 },
            scale: 1.0,
            primary: true,
            focused: true,
        }]
    }
}

#[async_trait]
impl Adapter for InMemoryAdapter {
    fn name(&self) -> &'static str {
        "in-memory"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut s = self.script.lock().await;
        s.launch_calls.push(spec.clone());
        s.next_launch_outcome.take().unwrap_or_else(|| Ok(Self::make_default_launch_outcome(4242)))
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        let mut s = self.script.lock().await;
        s.screenshot_calls.push(surface.id.clone());
        s.next_screenshot.take().unwrap_or_else(|| {
            Ok(Screenshot {
                png_bytes: minimal_png(),
                window_bounds_points: Rect { x: 0.0, y: 0.0, w: 800.0, h: 600.0 },
                content_bounds_points: None,
                scale: 2.0,
                captured_at_unix_ms: 0,
            })
        })
    }

    async fn key(&self, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.key_calls.push((surface.id.clone(), events.to_vec()));
        s.next_key_result.take().unwrap_or(Ok(()))
    }

    async fn text(&self, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.text_calls.push((surface.id.clone(), text.to_string()));
        s.next_text_result.take().unwrap_or(Ok(()))
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.click_calls.push((surface.id.clone(), spec.clone()));
        s.next_click_result.take().unwrap_or(Ok(()))
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.scroll_calls.push((surface.id.clone(), spec.clone()));
        s.next_scroll_result.take().unwrap_or(Ok(()))
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.close_calls.push(surface.id.clone());
        s.next_close_result.take().unwrap_or(Ok(()))
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.script.lock().await;
        s.focus_calls.push(surface.id.clone());
        s.next_focus_result.take().unwrap_or(Ok(()))
    }

    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
    ) -> Result<WaitOutcome, PortholeError> {
        let mut s = self.script.lock().await;
        s.wait_calls.push((surface.id.clone(), condition.clone()));
        s.next_wait_result.take().unwrap_or_else(|| {
            Ok(WaitOutcome {
                condition: wait_condition_tag(condition).to_string(),
                elapsed_ms: 0,
            })
        })
    }

    async fn wait_last_observed(
        &self,
        _surface: &SurfaceInfo,
        _condition: &WaitCondition,
    ) -> Result<LastObserved, PortholeError> {
        let mut s = self.script.lock().await;
        s.next_wait_last_observed.take().unwrap_or(Ok(LastObserved::Presence { alive: true }))
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        let mut s = self.script.lock().await;
        s.attention_calls += 1;
        s.next_attention.take().unwrap_or_else(|| Ok(Self::default_attention()))
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        let mut s = self.script.lock().await;
        s.displays_calls += 1;
        s.next_displays.take().unwrap_or_else(|| Ok(Self::default_displays()))
    }

    async fn permissions(&self) -> Result<Vec<PermissionStatus>, PortholeError> {
        let mut s = self.script.lock().await;
        s.permissions_calls += 1;
        s.next_permissions.take().unwrap_or(Ok(vec![]))
    }
}

fn wait_condition_tag(c: &WaitCondition) -> &'static str {
    match c {
        WaitCondition::Stable { .. } => "stable",
        WaitCondition::Dirty { .. } => "dirty",
        WaitCondition::Exists => "exists",
        WaitCondition::Gone => "gone",
        WaitCondition::TitleMatches { .. } => "title_matches",
    }
}

fn minimal_png() -> Vec<u8> {
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
    use std::time::Duration;

    use super::*;
    use crate::adapter::RequireConfidence;
    use crate::ErrorCode;

    #[tokio::test]
    async fn launch_records_call_and_returns_default_outcome() {
        let adapter = InMemoryAdapter::new();
        let spec = ProcessLaunchSpec {
            app: "/Applications/Test.app".to_string(),
            args: vec!["--help".to_string()],
            cwd: None,
            env: vec![],
            timeout: Duration::from_secs(5),
            require_confidence: RequireConfidence::Strong,
        };
        let outcome = adapter.launch_process(&spec).await.unwrap();
        assert_eq!(outcome.confidence, Confidence::Strong);
        assert_eq!(adapter.launch_calls().await.len(), 1);
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
        adapter.set_next_key_result(Err(PortholeError::new(ErrorCode::PermissionNeeded, "no ax"))).await;
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let err = adapter.key(&outcome.surface, &[]).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionNeeded);
    }

    #[tokio::test]
    async fn key_call_is_recorded() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        adapter
            .key(&outcome.surface, &[KeyEvent { key: "Enter".into(), modifiers: vec![] }])
            .await
            .unwrap();
        let calls = adapter.key_calls().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1[0].key, "Enter");
    }

    #[tokio::test]
    async fn attention_counter_increments() {
        let adapter = InMemoryAdapter::new();
        adapter.attention().await.unwrap();
        adapter.attention().await.unwrap();
        assert_eq!(adapter.attention_calls().await, 2);
    }

    #[tokio::test]
    async fn wait_returns_default_outcome_with_condition_tag() {
        let adapter = InMemoryAdapter::new();
        let outcome = InMemoryAdapter::make_default_launch_outcome(1);
        let result = adapter.wait(&outcome.surface, &WaitCondition::Exists).await.unwrap();
        assert_eq!(result.condition, "exists");
    }
}
