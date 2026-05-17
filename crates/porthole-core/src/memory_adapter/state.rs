use super::WindowSnapshot;
use crate::{
    ErrorCode, PortholeError,
    attention::CursorPos,
    content_rect::Descent,
    display::{DisplayInfo, Rect},
    surface::SurfaceId,
};

pub(super) struct State {
    pub windows: Vec<FakeWindow>,
    pub displays: Vec<DisplayInfo>,
    pub focused_pid: Option<u32>,
    pub cursor: CursorPos,
    pub title_bar_h: f64,
    pub accessibility_granted: bool,
    pub screen_recording_granted: bool,
    pub advertise_system_permission_prompt: bool,
    pub next_pid: u32,
    pub next_cg_window_id: u32,
}

pub(super) struct FakeWindow {
    pub id: SurfaceId,
    pub pid: u32,
    pub cg_window_id: u32,
    pub outer_rect: Rect,
    pub title: Option<String>,
    pub app_name: Option<String>,
    pub alive: bool,
    pub role_override: Option<String>,
    pub descent_override: Option<Descent>,
    pub content_rect_override: Option<Rect>,
}

impl State {
    pub fn find_alive_by_surface_id(&self, id: &SurfaceId) -> Result<&FakeWindow, PortholeError> {
        match self.windows.iter().find(|w| &w.id == id) {
            Some(w) if w.alive => Ok(w),
            Some(_) => Err(PortholeError::new(ErrorCode::SurfaceDead, format!("surface {id} was closed"))),
            None => Err(PortholeError::surface_not_found(id.as_str())),
        }
    }

    pub fn find_alive_by_surface_id_mut(&mut self, id: &SurfaceId) -> Result<&mut FakeWindow, PortholeError> {
        // Two-pass to avoid holding a mutable borrow across the error builder.
        let dead;
        let missing;
        match self.windows.iter().position(|w| &w.id == id) {
            Some(idx) => {
                if self.windows[idx].alive {
                    return Ok(&mut self.windows[idx]);
                }
                dead = true;
                missing = false;
            }
            None => {
                dead = false;
                missing = true;
            }
        }
        if dead {
            Err(PortholeError::new(ErrorCode::SurfaceDead, format!("surface {id} was closed")))
        } else if missing {
            Err(PortholeError::surface_not_found(id.as_str()))
        } else {
            unreachable!()
        }
    }

    /// Pick the display whose bounds has the largest intersection with `rect`;
    /// tie-break primary; final fallback first display.
    pub fn display_for_rect(&self, rect: Rect) -> &DisplayInfo {
        let mut best: Option<(&DisplayInfo, f64)> = None;
        for d in &self.displays {
            let area = intersection_area(d.bounds, rect);
            match best {
                Some((_, best_area)) if area <= best_area => {}
                _ => best = Some((d, area)),
            }
        }
        match best {
            Some((d, area)) if area > 0.0 => d,
            _ => self.displays.iter().find(|d| d.primary).unwrap_or(&self.displays[0]),
        }
    }

    pub fn mint_pid(&mut self) -> u32 {
        let p = self.next_pid;
        self.next_pid = self.next_pid.wrapping_add(1);
        p
    }

    pub fn mint_cg_window_id(&mut self) -> u32 {
        let c = self.next_cg_window_id;
        self.next_cg_window_id = self.next_cg_window_id.wrapping_add(1);
        c
    }

    pub fn window_snapshot(&self, id: &SurfaceId) -> Option<WindowSnapshot> {
        self.windows.iter().find(|w| &w.id == id).map(|w| WindowSnapshot {
            id: w.id.clone(),
            pid: w.pid,
            cg_window_id: w.cg_window_id,
            outer_rect: w.outer_rect,
            title: w.title.clone(),
            app_name: w.app_name.clone(),
            alive: w.alive,
        })
    }
}

fn intersection_area(a: Rect, b: Rect) -> f64 {
    let x1 = a.x.max(b.x);
    let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w);
    let y2 = (a.y + a.h).min(b.y + b.h);
    let w = (x2 - x1).max(0.0);
    let h = (y2 - y1).max(0.0);
    w * h
}
