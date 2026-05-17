use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;

use super::{
    MemoryAdapter,
    state::{FakeWindow, State},
    video::FakeVideoSession,
};
use crate::{
    ErrorCode, PortholeError,
    adapter::{
        Adapter, ArtifactLaunchSpec, Confidence, Correlation, LaunchOutcome, ProcessLaunchSpec, Rect, Screenshot,
        VideoCaptureFramePublisher, VideoCaptureSession,
    },
    attention::AttentionInfo,
    content_rect::{ContentRectInfo, Descent},
    display::DisplayInfo,
    input::{ClickSpec, KeyEvent, PointerMoveSpec, ScrollSpec},
    permission::{SystemPermissionPromptOutcome, SystemPermissionStatus},
    placement::GeometrySnapshot,
    search::{Candidate, SearchQuery, encode_ref},
    surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState},
    wait::{WaitCondition, WaitOutcome, WaitTimeout},
};

#[async_trait]
impl Adapter for MemoryAdapter {
    fn name(&self) -> &'static str {
        "memory"
    }

    async fn launch_process(&self, spec: &ProcessLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut s = self.state.lock().unwrap();
        let name = basename(&spec.app);
        let surface = synthesize_window(&mut s, Some(name.clone()), Some(name));
        Ok(LaunchOutcome {
            surface,
            confidence: Confidence::Strong,
            correlation: Correlation::Tag,
            surface_was_preexisting: false,
        })
    }

    async fn launch_artifact(&self, spec: &ArtifactLaunchSpec) -> Result<LaunchOutcome, PortholeError> {
        let mut s = self.state.lock().unwrap();
        let filename = spec.path.file_name().and_then(|n| n.to_str()).unwrap_or("artifact").to_string();
        let surface = synthesize_window(&mut s, Some(filename.clone()), Some(filename));
        Ok(LaunchOutcome {
            surface,
            confidence: Confidence::Strong,
            correlation: Correlation::DocumentMatch,
            surface_was_preexisting: false,
        })
    }

    async fn screenshot(&self, surface: &SurfaceInfo) -> Result<Screenshot, PortholeError> {
        let s = self.state.lock().unwrap();
        let w = s.find_alive_by_surface_id(&surface.id)?;
        let display = s.display_for_rect(w.outer_rect);
        Ok(Screenshot {
            png_bytes: minimal_png(),
            window_bounds_points: w.outer_rect,
            content_bounds_points: None,
            scale: display.scale,
            captured_at_unix_ms: 0,
        })
    }

    async fn start_video_capture(&self, _surface: &SurfaceInfo) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
        Ok(Box::new(FakeVideoSession::new()))
    }

    async fn start_video_capture_publisher(
        &self,
        surface: &SurfaceInfo,
        publisher: Arc<dyn VideoCaptureFramePublisher>,
    ) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
        // Validate the window is alive, then fire one canned frame to the
        // publisher *now*. The returned session yields None on next_frame.
        let _ = self.state.lock().unwrap().find_alive_by_surface_id(&surface.id)?;
        let frame = super::video::canned_frame();
        let _ = publisher.publish_frame(frame.as_view());
        Ok(Box::new(super::video::FakeVideoSession::exhausted()))
    }

    async fn key(&self, surface: &SurfaceInfo, _events: &[KeyEvent]) -> Result<(), PortholeError> {
        // No observable state for typed keys; the operation just checks the
        // surface is alive.
        let _ = self.state.lock().unwrap().find_alive_by_surface_id(&surface.id)?;
        Ok(())
    }

    async fn text(&self, surface: &SurfaceInfo, _text: &str) -> Result<(), PortholeError> {
        let _ = self.state.lock().unwrap().find_alive_by_surface_id(&surface.id)?;
        Ok(())
    }

    async fn click(&self, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
        // Move cursor to the window-local click position in global coords.
        let mut s = self.state.lock().unwrap();
        let outer = s.find_alive_by_surface_id(&surface.id)?.outer_rect;
        let display_id = s.display_for_rect(outer).id.clone();
        s.cursor.x = outer.x + spec.x;
        s.cursor.y = outer.y + spec.y;
        s.cursor.display_id = Some(display_id);
        Ok(())
    }

    async fn scroll(&self, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
        // Scroll positions the cursor at (x, y) before firing; wheel deltas
        // have no observable state beyond that.
        let mut s = self.state.lock().unwrap();
        let outer = s.find_alive_by_surface_id(&surface.id)?.outer_rect;
        let display_id = s.display_for_rect(outer).id.clone();
        s.cursor.x = outer.x + spec.x;
        s.cursor.y = outer.y + spec.y;
        s.cursor.display_id = Some(display_id);
        Ok(())
    }

    async fn pointer_move(&self, surface: &SurfaceInfo, spec: &PointerMoveSpec) -> Result<(), PortholeError> {
        let mut s = self.state.lock().unwrap();
        let outer = s.find_alive_by_surface_id(&surface.id)?.outer_rect;
        let display_id = s.display_for_rect(outer).id.clone();
        s.cursor.x = outer.x + spec.x;
        s.cursor.y = outer.y + spec.y;
        s.cursor.display_id = Some(display_id);
        Ok(())
    }

    async fn close(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.state.lock().unwrap();
        let pid = {
            let w = s.find_alive_by_surface_id_mut(&surface.id)?;
            w.alive = false;
            w.pid
        };
        if s.focused_pid == Some(pid) {
            s.focused_pid = None;
        }
        Ok(())
    }

    async fn focus(&self, surface: &SurfaceInfo) -> Result<(), PortholeError> {
        let mut s = self.state.lock().unwrap();
        let pid = s.find_alive_by_surface_id(&surface.id)?.pid;
        s.focused_pid = Some(pid);
        Ok(())
    }

    async fn wait(
        &self,
        surface: &SurfaceInfo,
        condition: &WaitCondition,
        _deadline: std::time::Instant,
    ) -> Result<WaitOutcome, WaitTimeout> {
        // Asymmetric on purpose: unlike every other state-validating method on
        // this fake, `wait` cannot return `SurfaceDead`. The trait signature
        // returns `Result<_, WaitTimeout>` (not `PortholeError`) — see
        // `Adapter::wait` — so there is no shape in which a dead-surface error
        // could surface here. We touch the state to keep parity with the
        // others (and to surface lock-poisoning if it happened) but discard
        // the result. Tests asserting dead-window behaviour reach this code
        // path through `HandleStore::require_alive` in the daemon pipeline,
        // not through the adapter directly.
        let _ = self.state.lock().unwrap().find_alive_by_surface_id(&surface.id);
        Ok(WaitOutcome {
            condition: wait_condition_tag(condition).to_string(),
            elapsed_ms: 0,
        })
    }

    async fn attention(&self) -> Result<AttentionInfo, PortholeError> {
        let s = self.state.lock().unwrap();
        let (focused_surface_id, focused_app_name, focused_display_id) = match s.focused_pid {
            Some(pid) => match s.windows.iter().find(|w| w.alive && w.pid == pid) {
                Some(w) => {
                    let display = s.display_for_rect(w.outer_rect);
                    (Some(w.id.clone()), w.app_name.clone(), Some(display.id.clone()))
                }
                None => (None, None, None),
            },
            None => (None, None, None),
        };
        Ok(AttentionInfo {
            focused_surface_id,
            focused_app_name,
            focused_display_id,
            cursor: s.cursor.clone(),
            recently_active_surface_ids: vec![],
        })
    }

    async fn frontmost_window_id(&self) -> Result<Option<u32>, PortholeError> {
        let s = self.state.lock().unwrap();
        Ok(match s.focused_pid {
            Some(pid) => s.windows.iter().find(|w| w.alive && w.pid == pid).map(|w| w.cg_window_id),
            None => None,
        })
    }

    async fn displays(&self) -> Result<Vec<DisplayInfo>, PortholeError> {
        Ok(self.state.lock().unwrap().displays.clone())
    }

    async fn system_permissions(&self) -> Result<Vec<SystemPermissionStatus>, PortholeError> {
        let s = self.state.lock().unwrap();
        Ok(vec![
            SystemPermissionStatus {
                name: "accessibility".into(),
                granted: s.accessibility_granted,
                purpose: "input injection and some wait conditions".into(),
            },
            SystemPermissionStatus {
                name: "screen_recording".into(),
                granted: s.screen_recording_granted,
                purpose: "window screenshot capture, frame-diff waits, and live capture sessions".into(),
            },
        ])
    }

    async fn request_system_permission_prompt(&self, name: &str) -> Result<SystemPermissionPromptOutcome, PortholeError> {
        let s = self.state.lock().unwrap();
        let granted = match name {
            "accessibility" => s.accessibility_granted,
            "screen_recording" => s.screen_recording_granted,
            _ => {
                return Err(PortholeError::new(
                    ErrorCode::InvalidArgument,
                    format!("unknown permission name '{name}'; expected one of: accessibility, screen_recording"),
                ));
            }
        };
        Ok(SystemPermissionPromptOutcome {
            permission: name.to_string(),
            granted_before: granted,
            granted_after: granted,
            requires_daemon_restart: false,
            notes: String::new(),
        })
    }

    async fn ensure_system_permission(&self, name: &str) -> Result<(), PortholeError> {
        let s = self.state.lock().unwrap();
        let granted = match name {
            "accessibility" => s.accessibility_granted,
            "screen_recording" => s.screen_recording_granted,
            _ => return Ok(()),
        };
        if granted {
            Ok(())
        } else {
            Err(PortholeError::new(
                ErrorCode::SystemPermissionNeeded,
                format!("permission '{name}' is not granted"),
            ))
        }
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
        let title_re = match &query.title_pattern {
            Some(p) => Some(
                Regex::new(p).map_err(|e| PortholeError::new(ErrorCode::InvalidArgument, format!("invalid title_pattern regex: {e}")))?,
            ),
            None => None,
        };
        let s = self.state.lock().unwrap();
        let mut out: Vec<Candidate> = s
            .windows
            .iter()
            .filter(|w| w.alive && matches_query(w, query, title_re.as_ref(), s.focused_pid))
            .map(|w| Candidate {
                ref_: encode_ref(w.pid, w.cg_window_id),
                app_name: w.app_name.clone(),
                title: w.title.clone(),
                pid: w.pid,
                cg_window_id: w.cg_window_id,
            })
            .collect();
        // frontmost: when explicitly true, `matches_query` already filtered to
        // the focused pid. If that pid owns multiple windows, we truncate to
        // one — but note the surviving window is insertion-order-first, not a
        // semantic "frontmost window within app". The real macOS adapter has a
        // meaningful ordering here; tests that depend on it should configure
        // a single window per focused pid until this fake grows a real
        // z-order.
        if matches!(query.frontmost, Some(true)) && out.len() > 1 {
            out.truncate(1);
        }
        Ok(out)
    }

    async fn window_alive(&self, pid: u32, cg_window_id: u32) -> Result<Option<SurfaceInfo>, PortholeError> {
        let s = self.state.lock().unwrap();
        Ok(s.windows
            .iter()
            .find(|w| w.alive && w.pid == pid && w.cg_window_id == cg_window_id)
            .map(|w| SurfaceInfo {
                id: w.id.clone(),
                kind: SurfaceKind::Window,
                state: SurfaceState::Alive,
                title: w.title.clone(),
                app_name: w.app_name.clone(),
                pid: Some(w.pid),
                parent_surface_id: None,
                cg_window_id: Some(w.cg_window_id),
            }))
    }

    async fn place_surface(&self, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError> {
        let mut s = self.state.lock().unwrap();
        let w = s.find_alive_by_surface_id_mut(&surface.id)?;
        w.outer_rect = rect;
        Ok(())
    }

    async fn snapshot_geometry(&self, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError> {
        let s = self.state.lock().unwrap();
        let w = s.find_alive_by_surface_id(&surface.id)?;
        let display = s.display_for_rect(w.outer_rect);
        let display_local = Rect {
            x: w.outer_rect.x - display.bounds.x,
            y: w.outer_rect.y - display.bounds.y,
            w: w.outer_rect.w,
            h: w.outer_rect.h,
        };
        Ok(GeometrySnapshot {
            display_id: display.id.clone(),
            display_local,
        })
    }

    async fn content_rect(&self, surface: &SurfaceInfo) -> Result<ContentRectInfo, PortholeError> {
        let s = self.state.lock().unwrap();
        let w = s.find_alive_by_surface_id(&surface.id)?;
        let rect = w.content_rect_override.unwrap_or_else(|| Rect {
            x: 0.0,
            y: s.title_bar_h,
            w: w.outer_rect.w,
            h: (w.outer_rect.h - s.title_bar_h).max(0.0),
        });
        let role = w.role_override.clone().unwrap_or_else(|| "AXScrollArea".to_string());
        let descent = w.descent_override.unwrap_or(Descent::Contents);
        Ok(ContentRectInfo { rect, role, descent })
    }

    fn capabilities(&self) -> Vec<&'static str> {
        let mut caps = vec![
            "launch_process",
            "screenshot",
            "input_key",
            "input_text",
            "input_click",
            "input_scroll",
            "input_pointer_move",
            "wait",
            "close",
            "focus",
            "attention",
            "attention_cursor",
            "attention_focused_app",
            "attention_focused_display",
            "attention_focused_surface",
            "displays",
            "search",
            "track",
            "launch_artifact",
            "placement",
            "replace",
            "auto_dismiss",
            "content_rect",
        ];
        if self.state.lock().unwrap().advertise_system_permission_prompt {
            caps.push("system_permission_prompt");
        }
        caps
    }
}

/// Add a window to state with synthesized pid + cg_window_id, focus it, and
/// return the SurfaceInfo for the freshly-tracked window. Used by
/// `launch_process` and `launch_artifact`.
fn synthesize_window(s: &mut State, title: Option<String>, app_name: Option<String>) -> SurfaceInfo {
    let id = SurfaceId::new();
    let pid = s.mint_pid();
    let cg_window_id = s.mint_cg_window_id();
    // Default new windows to an 800x600 centred-ish placement.
    let outer_rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };
    s.windows.push(FakeWindow {
        id: id.clone(),
        pid,
        cg_window_id,
        outer_rect,
        title: title.clone(),
        app_name: app_name.clone(),
        alive: true,
        role_override: None,
        descent_override: None,
        content_rect_override: None,
    });
    s.focused_pid = Some(pid);
    SurfaceInfo {
        id,
        kind: SurfaceKind::Window,
        state: SurfaceState::Alive,
        title,
        app_name,
        pid: Some(pid),
        parent_surface_id: None,
        cg_window_id: Some(cg_window_id),
    }
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn minimal_png() -> Vec<u8> {
    // 1x1 transparent PNG; same canned payload as in_memory.rs to avoid
    // re-rendering surprises for tests that pattern-match the magic header.
    // Keep in sync with `crates/porthole-core/src/in_memory.rs::minimal_png`.
    const BYTES: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x62,
        0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60,
        0x82,
    ];
    BYTES.to_vec()
}

fn matches_query(w: &FakeWindow, q: &SearchQuery, title_re: Option<&Regex>, focused_pid: Option<u32>) -> bool {
    if let Some(name) = &q.app_name {
        if w.app_name.as_deref() != Some(name) {
            return false;
        }
    }
    if let Some(re) = title_re {
        let title = w.title.as_deref().unwrap_or("");
        if !re.is_match(title) {
            return false;
        }
    }
    if !q.pids.is_empty() && !q.pids.contains(&w.pid) {
        return false;
    }
    if !q.cg_window_ids.is_empty() && !q.cg_window_ids.contains(&w.cg_window_id) {
        return false;
    }
    if matches!(q.frontmost, Some(true)) && focused_pid != Some(w.pid) {
        return false;
    }
    true
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
