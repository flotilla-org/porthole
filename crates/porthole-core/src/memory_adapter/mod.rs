//! In-memory test adapter with **real internal state**.
//!
//! Coexists with [`crate::in_memory::InMemoryAdapter`]. Where `InMemoryAdapter`
//! scripts return values per method call ("next click result is ...") and
//! records calls for after-the-fact assertions ("was click called?"),
//! `MemoryAdapter` holds a small in-memory desktop:
//!
//! - a list of windows (each with a real outer rect, pid, cg_window_id, alive
//!   flag, optional content-rect overrides)
//! - a list of displays (each with bounds + scale)
//! - a focus pointer
//! - a cursor position
//! - accessibility / screen-recording grant flags
//!
//! Operations *mutate* state. `place_surface` writes the new outer rect;
//! `pointer_move` updates the cursor; `focus` updates the focused pid; `close`
//! marks the window dead. Tests assert against state via
//! [`MemoryAdapter::window`] et al., not against "was X called with these
//! args".
//!
//! See `docs/adr/0003-stateful-test-fake.md` for the rationale and the planned
//! migration window.

mod adapter;
mod builder;
mod state;
#[cfg(test)]
mod tests;
mod video;

use std::sync::{Arc, Mutex};

pub use builder::{MemoryAdapterBuilder, WindowSpec};
use state::State;

use crate::{attention::CursorPos, display::Rect, surface::SurfaceId};

/// In-memory test adapter with stateful semantics. Implements [`Adapter`] for
/// every method; ops without natural state effects (key, text, scroll wheel
/// deltas) are no-ops on a live window. See module-level docs.
pub struct MemoryAdapter {
    state: Arc<Mutex<State>>,
}

impl MemoryAdapter {
    pub fn builder() -> MemoryAdapterBuilder {
        MemoryAdapterBuilder::new()
    }

    /// Snapshot a window's current state, including its alive flag.
    /// Returns `None` if no window with this id was ever in state.
    pub fn window(&self, id: &SurfaceId) -> Option<WindowSnapshot> {
        self.state.lock().unwrap().window_snapshot(id)
    }

    /// Snapshot the currently-focused pid.
    pub fn focused_pid(&self) -> Option<u32> {
        self.state.lock().unwrap().focused_pid
    }

    /// Snapshot the cursor position.
    pub fn cursor(&self) -> CursorPos {
        self.state.lock().unwrap().cursor.clone()
    }

    /// Number of windows currently alive in state.
    pub fn live_window_count(&self) -> usize {
        self.state.lock().unwrap().windows.iter().filter(|w| w.alive).count()
    }
}

/// Lock-free, owned view of a window's state. Returned by
/// [`MemoryAdapter::window`] for test assertions.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowSnapshot {
    pub id: SurfaceId,
    pub pid: u32,
    pub cg_window_id: u32,
    pub outer_rect: Rect,
    pub title: Option<String>,
    pub app_name: Option<String>,
    pub alive: bool,
}
