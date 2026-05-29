pub mod bridge;

mod snapshot;

use std::time::{Duration, Instant};

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{Adapter, ArtifactLaunchSpec, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot, VideoCaptureSession},
    attention::{AttentionInfo, CursorPos},
    content_rect::ContentRectInfo,
    display::{DisplayId, DisplayInfo},
    input::{ClickSpec, KeyEvent, PointerMoveSpec, ScrollSpec},
    permission::{SystemPermissionPromptOutcome, SystemPermissionStatus},
    placement::GeometrySnapshot,
    search::{Candidate, SearchQuery, encode_ref},
    surface::{PlatformSurfaceRef, SurfaceId, SurfaceInfo},
    wait::{LastObserved, WAIT_SAMPLE_INTERVAL, WaitCondition, WaitOutcome, WaitTimeout},
};
use serde::Deserialize;
use serde_json::json;
use tokio::time::sleep;

use crate::{
    bridge::KWinBridge,
    snapshot::{KWinSnapshotPayload, KWinWindow},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(1);
const COMMAND_POLL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug)]
pub struct KWinAdapter {
    bridge: KWinBridge,
}

#[derive(Debug, Deserialize)]
struct CommandResult {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

impl KWinAdapter {
    pub fn new(bridge: KWinBridge) -> Self {
        Self { bridge }
    }

    pub fn bridge(&self) -> KWinBridge {
        self.bridge.clone()
    }

    fn no_snapshot_error() -> PortholeError {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "KWin adapter has not received a compositor snapshot; install and enable the porthole KWin control script",
        )
    }

    async fn snapshot(&self) -> Result<KWinSnapshotPayload, PortholeError> {
        let snapshot = self.bridge.latest_snapshot().await.ok_or_else(Self::no_snapshot_error)?;
        let snapshot: KWinSnapshotPayload = serde_json::from_value(snapshot.payload)
            .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("invalid KWin snapshot payload: {error}")))?;
        if snapshot.schema_version != 1 {
            return Err(PortholeError::new(
                ErrorCode::InternalError,
                format!("unsupported KWin snapshot schema version {}", snapshot.schema_version),
            ));
        }
        Ok(snapshot)
    }

    async fn find_window(&self, platform_ref: &PlatformSurfaceRef) -> Result<Option<KWinWindow>, PortholeError> {
        let PlatformSurfaceRef::Kwin { window_id } = platform_ref else {
            return Ok(None);
        };
        Ok(self
            .snapshot()
            .await?
            .windows
            .into_iter()
            .find(|window| &window.window_id == window_id))
    }

    async fn run_window_command(&self, kind: &str, window_id: String, payload: serde_json::Value) -> Result<(), PortholeError> {
        let command = self
            .bridge
            .queue_command(
                kind,
                json!({
                    "windowId": window_id,
                    "args": payload,
                }),
            )
            .await;
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            if let Some(completion) = self.bridge.completion(&command.command_id).await {
                let result: CommandResult = serde_json::from_str(&completion.result_json).map_err(|error| {
                    PortholeError::new(
                        ErrorCode::InternalError,
                        format!("KWin command {} returned invalid completion JSON: {error}", command.command_id),
                    )
                })?;
                if result.ok {
                    return Ok(());
                }
                return Err(PortholeError::new(
                    ErrorCode::InternalError,
                    format!(
                        "KWin command {kind} failed: {}",
                        result.error.unwrap_or_else(|| "unknown error".to_string())
                    ),
                ));
            }
            if Instant::now() >= deadline {
                return Err(PortholeError::new(
                    ErrorCode::InternalError,
                    format!("KWin command {kind} timed out waiting for the control script"),
                ));
            }
            sleep(COMMAND_POLL).await;
        }
    }
}

fn surface_from_window(window: &KWinWindow) -> SurfaceInfo {
    let mut surface = SurfaceInfo::window(SurfaceId::new(), window.pid);
    surface.title = window.caption.clone();
    surface.app_name = window.app_name();
    surface.platform_ref = Some(PlatformSurfaceRef::kwin(window.window_id.clone()));
    surface
}

fn candidate_from_window(window: &KWinWindow) -> Candidate {
    let platform_ref = PlatformSurfaceRef::kwin(window.window_id.clone());
    Candidate {
        ref_: encode_ref(window.pid, platform_ref.clone()),
        app_name: window.app_name(),
        title: window.caption.clone(),
        pid: window.pid,
        platform_ref,
    }
}

fn window_matches_query(
    window: &KWinWindow,
    query: &SearchQuery,
    title_regex: Option<&regex::Regex>,
    active_window_id: Option<&str>,
) -> bool {
    if !window.normal_window {
        return false;
    }
    if let Some(app_name) = &query.app_name {
        let app_name_lc = app_name.to_lowercase();
        let matches_app = window
            .app_name()
            .is_some_and(|candidate| candidate.to_lowercase().contains(&app_name_lc));
        if !matches_app {
            return false;
        }
    }
    if let Some(regex) = title_regex {
        if !window.caption.as_deref().is_some_and(|title| regex.is_match(title)) {
            return false;
        }
    }
    if !query.pids.is_empty() && !query.pids.contains(&window.pid) {
        return false;
    }
    if !query.platform_refs.is_empty() && !query.platform_refs.contains(&PlatformSurfaceRef::kwin(window.window_id.clone())) {
        return false;
    }
    if query.frontmost == Some(true) && active_window_id != Some(window.window_id.as_str()) {
        return false;
    }
    true
}

fn unsupported(message: &str) -> PortholeError {
    PortholeError::new(ErrorCode::AdapterUnsupported, message)
}

#[async_trait]
impl Adapter for KWinAdapter {
    fn name(&self) -> &'static str {
        "kwin"
    }

    async fn launch_process(&self, _spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        Err(unsupported("KWin adapter does not launch processes in this branch"))
    }

    async fn screenshot(&self, _surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        Err(unsupported("KWin adapter does not support screenshots yet"))
    }

    async fn start_video_capture(&self, _surface: &SurfaceInfo) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
        Err(unsupported("KWin adapter does not support recording yet"))
    }

    async fn key(&self, _surface: &SurfaceInfo, _events: &[KeyEvent]) -> Result<(), PortholeError> {
        Err(unsupported("KWin adapter does not support keyboard input yet"))
    }

    async fn text(&self, _surface: &SurfaceInfo, _text: &str) -> Result<(), PortholeError> {
        Err(unsupported("KWin adapter does not support text input yet"))
    }

    async fn click(&self, _surface: &SurfaceInfo, _spec: &ClickSpec) -> Result<(), PortholeError> {
        Err(unsupported("KWin adapter does not support pointer input yet"))
    }

    async fn scroll(&self, _surface: &SurfaceInfo, _spec: &ScrollSpec) -> Result<(), PortholeError> {
        Err(unsupported("KWin adapter does not support pointer input yet"))
    }

    async fn pointer_move(&self, _surface: &SurfaceInfo, _spec: &PointerMoveSpec) -> Result<(), PortholeError> {
        Err(unsupported("KWin adapter does not support pointer input yet"))
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let Some(PlatformSurfaceRef::Kwin { window_id }) = &surface.platform_ref else {
            return Err(PortholeError::new(ErrorCode::InvalidArgument, "surface is not a KWin surface"));
        };
        self.run_window_command("close", window_id.clone(), json!({})).await
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let Some(PlatformSurfaceRef::Kwin { window_id }) = &surface.platform_ref else {
            return Err(PortholeError::new(ErrorCode::InvalidArgument, "surface is not a KWin surface"));
        };
        self.run_window_command("focus", window_id.clone(), json!({})).await
    }

    async fn wait(&self, surface: &SurfaceInfo, condition: &WaitCondition, deadline: Instant) -> Result<WaitOutcome, WaitTimeout> {
        let start = Instant::now();
        let mut last_observed = surface.title.clone();
        let title_regex = match condition {
            WaitCondition::TitleMatches { pattern } => Some(regex::Regex::new(pattern).map_err(|_| WaitTimeout {
                last_observed: LastObserved::Title {
                    title: last_observed.clone(),
                },
                elapsed_ms: start.elapsed().as_millis() as u64,
            })?),
            _ => None,
        };
        loop {
            let alive = match &surface.platform_ref {
                Some(platform_ref) => self.find_window(platform_ref).await.map(|window| window.is_some()).unwrap_or(false),
                None => false,
            };
            let matched = match condition {
                WaitCondition::Exists => alive,
                WaitCondition::Gone => !alive,
                WaitCondition::TitleMatches { .. } => {
                    let regex = title_regex.as_ref().expect("compiled title regex");
                    match &surface.platform_ref {
                        Some(platform_ref) => match self.find_window(platform_ref).await {
                            Ok(Some(window)) => {
                                last_observed = window.caption.clone();
                                window.caption.as_deref().is_some_and(|title| regex.is_match(title))
                            }
                            _ => false,
                        },
                        None => false,
                    }
                }
                WaitCondition::Stable { .. } | WaitCondition::Dirty { .. } => false,
            };
            if matched {
                return Ok(WaitOutcome {
                    condition: match condition {
                        WaitCondition::Stable { .. } => "stable",
                        WaitCondition::Dirty { .. } => "dirty",
                        WaitCondition::Exists => "exists",
                        WaitCondition::Gone => "gone",
                        WaitCondition::TitleMatches { .. } => "title_matches",
                    }
                    .to_string(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
            }
            if Instant::now() >= deadline {
                return Err(WaitTimeout {
                    last_observed: match condition {
                        WaitCondition::TitleMatches { .. } => LastObserved::Title { title: last_observed },
                        WaitCondition::Stable { .. } | WaitCondition::Dirty { .. } => LastObserved::FrameChange {
                            last_change_ms_ago: start.elapsed().as_millis() as u64,
                            last_change_pct: 0.0,
                        },
                        WaitCondition::Exists | WaitCondition::Gone => LastObserved::Presence { alive },
                    },
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
            }
            sleep(WAIT_SAMPLE_INTERVAL).await;
        }
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        let snapshot = self.snapshot().await?;
        let active = snapshot.active_window.as_ref();
        Ok(AttentionInfo {
            focused_surface_id: None,
            focused_app_name: active.and_then(KWinWindow::app_name),
            focused_display_id: active.and_then(|window| window.output.clone()).map(DisplayId::new),
            cursor: CursorPos {
                x: snapshot.cursor.as_ref().map_or(0.0, |cursor| cursor.x),
                y: snapshot.cursor.as_ref().map_or(0.0, |cursor| cursor.y),
                display_id: snapshot
                    .cursor
                    .as_ref()
                    .and_then(|cursor| cursor.output.clone())
                    .map(DisplayId::new),
            },
            recently_active_surface_ids: vec![],
        })
    }

    async fn focused_platform_surface_ref(&self) -> Result<Option<PlatformSurfaceRef>, PortholeError> {
        let snapshot = self.snapshot().await?;
        Ok(snapshot
            .active_window
            .or_else(|| snapshot.windows.into_iter().find(|window| window.active))
            .map(|window| PlatformSurfaceRef::kwin(window.window_id)))
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        Ok(self.snapshot().await?.displays())
    }

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError> {
        Ok(vec![])
    }

    async fn request_system_permission_prompt(&self, _name: &str) -> Result<SystemPermissionPromptOutcome, PortholeError> {
        Err(unsupported("KWin adapter has no system permission prompt integration"))
    }

    async fn ensure_system_permission(&self, _name: &str) -> Result<(), PortholeError> {
        Ok(())
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
        let title_regex = query
            .title_pattern
            .as_deref()
            .map(regex::Regex::new)
            .transpose()
            .map_err(|error| PortholeError::new(ErrorCode::InvalidArgument, format!("invalid title_pattern regex: {error}")))?;
        let snapshot = self.snapshot().await?;
        let active_window_id = snapshot.active_window.as_ref().map(|window| window.window_id.as_str());
        Ok(snapshot
            .windows
            .iter()
            .filter(|window| window_matches_query(window, query, title_regex.as_ref(), active_window_id))
            .map(candidate_from_window)
            .collect())
    }

    async fn surface_alive(&self, pid: u32, platform_ref: &PlatformSurfaceRef) -> Result<Option<SurfaceInfo>, PortholeError> {
        Ok(self
            .find_window(platform_ref)
            .await?
            .filter(|window| window.pid == pid && window.normal_window)
            .map(|window| surface_from_window(&window)))
    }

    async fn launch_artifact(&self, _spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        Err(unsupported("KWin adapter does not launch artifacts in this branch"))
    }

    async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError> {
        let Some(PlatformSurfaceRef::Kwin { window_id }) = &surface.platform_ref else {
            return Err(PortholeError::new(ErrorCode::InvalidArgument, "surface is not a KWin surface"));
        };
        self.run_window_command(
            "place_surface",
            window_id.clone(),
            json!({ "x": rect.x, "y": rect.y, "width": rect.w, "height": rect.h }),
        )
        .await
    }

    async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError> {
        let Some(platform_ref) = &surface.platform_ref else {
            return Err(PortholeError::new(ErrorCode::InvalidArgument, "surface has no platform_ref"));
        };
        let window = self
            .find_window(platform_ref)
            .await?
            .ok_or_else(|| PortholeError::new(ErrorCode::SurfaceDead, "KWin surface is no longer alive"))?;
        let rect = window
            .frame_geometry
            .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "KWin snapshot did not include frame geometry"))?;
        let display_id = window.output.map(DisplayId::new).unwrap_or_else(|| DisplayId::new("kwin"));
        Ok(GeometrySnapshot {
            display_id,
            display_local: Rect {
                x: rect.x,
                y: rect.y,
                w: rect.width,
                h: rect.height,
            },
        })
    }

    async fn content_rect(&self, _surface: &SurfaceInfo) -> Result<ContentRectInfo, PortholeError> {
        Err(unsupported("KWin adapter does not support content rects yet"))
    }

    fn capabilities(&self) -> Vec<&'static str> {
        vec![
            "search",
            "wait",
            "close",
            "focus",
            "attention",
            "attention_cursor",
            "attention_focused_app",
            "attention_focused_display",
            "attention_focused_surface",
            "displays",
            "place_surface",
            "snapshot_geometry",
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use porthole_core::{adapter::Adapter, search::SearchQuery};

    use super::*;

    const SNAPSHOT: &str = r#"{
        "schemaVersion": 1,
        "activeWindow": {
            "windowId": "win-a",
            "caption": "Terminal",
            "resourceClass": "org.kde.konsole",
            "pid": 123,
            "normalWindow": true,
            "active": true,
            "output": "eDP-1",
            "frameGeometry": { "x": 10, "y": 20, "width": 800, "height": 600 }
        },
        "cursor": { "x": 30, "y": 40, "output": "eDP-1" },
        "outputs": [
            { "name": "eDP-1", "geometry": { "x": 0, "y": 0, "width": 1920, "height": 1080 }, "scale": 1, "active": true }
        ],
        "windowCount": 2,
        "windows": [
            {
                "windowId": "win-a",
                "caption": "Terminal",
                "resourceClass": "org.kde.konsole",
                "pid": 123,
                "normalWindow": true,
                "active": true,
                "output": "eDP-1",
                "frameGeometry": { "x": 10, "y": 20, "width": 800, "height": 600 }
            },
            {
                "windowId": "win-b",
                "caption": "Browser",
                "resourceClass": "firefox",
                "pid": 456,
                "normalWindow": true,
                "active": false,
                "output": "eDP-1",
                "frameGeometry": { "x": 100, "y": 200, "width": 1024, "height": 768 }
            }
        ]
    }"#;

    async fn adapter_with_snapshot() -> KWinAdapter {
        let bridge = KWinBridge::new();
        bridge.publish_snapshot_json(SNAPSHOT).await.unwrap();
        KWinAdapter::new(bridge)
    }

    #[tokio::test]
    async fn search_filters_snapshot_windows_and_encodes_kwin_refs() {
        let adapter = adapter_with_snapshot().await;

        let candidates = adapter
            .search(&SearchQuery {
                app_name: Some("konsole".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].platform_ref, PlatformSurfaceRef::kwin("win-a"));
        assert!(candidates[0].ref_.starts_with("ref_"));
    }

    #[tokio::test]
    async fn surface_alive_returns_matching_kwin_surface() {
        let adapter = adapter_with_snapshot().await;

        let surface = adapter
            .surface_alive(123, &PlatformSurfaceRef::kwin("win-a"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(surface.pid, Some(123));
        assert_eq!(surface.title.as_deref(), Some("Terminal"));
        assert_eq!(surface.platform_ref, Some(PlatformSurfaceRef::kwin("win-a")));
    }

    #[tokio::test]
    async fn attention_and_displays_come_from_snapshot() {
        let adapter = adapter_with_snapshot().await;

        let attention = adapter.attention().await.unwrap();
        let displays = adapter.displays().await.unwrap();

        assert_eq!(attention.focused_app_name.as_deref(), Some("org.kde.konsole"));
        assert_eq!(attention.cursor.x, 30.0);
        assert_eq!(displays.len(), 1);
        assert_eq!(displays[0].id.as_str(), "eDP-1");
    }

    #[tokio::test]
    async fn focus_queues_command_and_waits_for_completion() {
        let adapter = adapter_with_snapshot().await;
        let bridge = adapter.bridge();
        let mut surface = SurfaceInfo::window(SurfaceId::new(), 123);
        surface.platform_ref = Some(PlatformSurfaceRef::kwin("win-a"));
        let script = tokio::spawn(async move {
            let command_json = loop {
                if let Some(command_json) = bridge.next_command_json("test").await.unwrap() {
                    break command_json;
                }
                sleep(Duration::from_millis(1)).await;
            };
            let command: bridge::KWinCommand = serde_json::from_str(&command_json).unwrap();
            assert_eq!(command.kind, "focus");
            assert_eq!(command.payload["windowId"], "win-a");
            bridge.complete_command_json(&command.command_id, r#"{"ok":true}"#).await;
        });

        adapter.focus(&surface).await.unwrap();
        script.await.unwrap();
    }

    #[tokio::test]
    async fn adapter_is_object_safe() {
        let adapter: Arc<dyn Adapter> = Arc::new(adapter_with_snapshot().await);
        assert_eq!(adapter.name(), "kwin");
    }
}
