use std::sync::{Arc, Mutex};

use super::{
    MemoryAdapter,
    state::{FakeWindow, State},
};
use crate::{
    attention::CursorPos,
    content_rect::Descent,
    display::{DisplayInfo, Rect},
    surface::SurfaceId,
};

/// Per-window setup for [`MemoryAdapterBuilder::window`]. Only `pid`,
/// `cg_window_id`, and `outer_rect` are required; overrides default to None
/// and `content_rect` is derived from `outer_rect` + the adapter's
/// `title_bar_h`.
#[derive(Clone, Debug)]
pub struct WindowSpec {
    pub pid: u32,
    pub cg_window_id: u32,
    pub outer_rect: Rect,
    pub title: Option<String>,
    pub app_name: Option<String>,
    pub role_override: Option<String>,
    pub descent_override: Option<Descent>,
    pub content_rect_override: Option<Rect>,
}

impl WindowSpec {
    pub fn new(pid: u32, cg_window_id: u32, outer_rect: Rect) -> Self {
        Self {
            pid,
            cg_window_id,
            outer_rect,
            title: None,
            app_name: None,
            role_override: None,
            descent_override: None,
            content_rect_override: None,
        }
    }

    pub fn with_title(mut self, t: impl Into<String>) -> Self {
        self.title = Some(t.into());
        self
    }
    pub fn with_app_name(mut self, a: impl Into<String>) -> Self {
        self.app_name = Some(a.into());
        self
    }
    pub fn with_role(mut self, r: impl Into<String>) -> Self {
        self.role_override = Some(r.into());
        self
    }
    pub fn with_descent(mut self, d: Descent) -> Self {
        self.descent_override = Some(d);
        self
    }
    pub fn with_content_rect_override(mut self, r: Rect) -> Self {
        self.content_rect_override = Some(r);
        self
    }
}

/// Builder for [`MemoryAdapter`]. See module docs for the design.
pub struct MemoryAdapterBuilder {
    displays: Vec<DisplayInfo>,
    windows: Vec<(SurfaceId, WindowSpec)>,
    focused_pid: Option<u32>,
    cursor: CursorPos,
    title_bar_h: f64,
    accessibility_granted: bool,
    screen_recording_granted: bool,
    advertise_system_permission_prompt: bool,
    next_pid: u32,
    next_cg_window_id: u32,
}

impl MemoryAdapterBuilder {
    pub(super) fn new() -> Self {
        Self {
            displays: Vec::new(),
            windows: Vec::new(),
            focused_pid: None,
            cursor: CursorPos {
                x: 0.0,
                y: 0.0,
                display_id: None,
            },
            title_bar_h: 28.0,
            accessibility_granted: true,
            screen_recording_granted: true,
            advertise_system_permission_prompt: false,
            next_pid: 10_000,
            next_cg_window_id: 1_000,
        }
    }

    pub fn display(mut self, d: DisplayInfo) -> Self {
        self.displays.push(d);
        self
    }

    /// Add a window with the given spec. Returns the freshly-minted
    /// [`SurfaceId`] so callers can drive ops against it.
    ///
    /// Unlike every other builder method, this takes `&mut self` rather than
    /// `self` because it must hand back the new `SurfaceId` — a `self → Self`
    /// signature can't return both. The intended pattern is:
    /// `let mut b = MemoryAdapter::builder().display(...); let id = b.window(...); let a = b.build();`
    pub fn window(&mut self, w: WindowSpec) -> SurfaceId {
        let id = SurfaceId::new();
        self.windows.push((id.clone(), w));
        id
    }

    pub fn focus(mut self, pid: u32) -> Self {
        self.focused_pid = Some(pid);
        self
    }

    pub fn cursor(mut self, c: CursorPos) -> Self {
        self.cursor = c;
        self
    }

    pub fn title_bar_h(mut self, h: f64) -> Self {
        self.title_bar_h = h;
        self
    }

    pub fn accessibility_granted(mut self, granted: bool) -> Self {
        self.accessibility_granted = granted;
        self
    }

    pub fn screen_recording_granted(mut self, granted: bool) -> Self {
        self.screen_recording_granted = granted;
        self
    }

    pub fn advertise_system_permission_prompt(mut self, on: bool) -> Self {
        self.advertise_system_permission_prompt = on;
        self
    }

    pub fn build(self) -> MemoryAdapter {
        // Default to one 1920x1080 logical display at scale=1.0 if none configured.
        // Realism: the adapter would never see "no displays".
        let displays = if self.displays.is_empty() {
            vec![DisplayInfo {
                id: crate::display::DisplayId::new("mem-display-0"),
                bounds: Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1920.0,
                    h: 1080.0,
                },
                scale: 1.0,
                primary: true,
                focused: true,
            }]
        } else {
            self.displays
        };

        let windows = self
            .windows
            .into_iter()
            .map(|(id, spec)| FakeWindow {
                id,
                pid: spec.pid,
                cg_window_id: spec.cg_window_id,
                outer_rect: spec.outer_rect,
                title: spec.title,
                app_name: spec.app_name,
                alive: true,
                role_override: spec.role_override,
                descent_override: spec.descent_override,
                content_rect_override: spec.content_rect_override,
            })
            .collect();

        let state = State {
            windows,
            displays,
            focused_pid: self.focused_pid,
            cursor: self.cursor,
            title_bar_h: self.title_bar_h,
            accessibility_granted: self.accessibility_granted,
            screen_recording_granted: self.screen_recording_granted,
            advertise_system_permission_prompt: self.advertise_system_permission_prompt,
            next_pid: self.next_pid,
            next_cg_window_id: self.next_cg_window_id,
        };
        MemoryAdapter {
            state: Arc::new(Mutex::new(state)),
        }
    }
}
