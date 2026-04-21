#![cfg(target_os = "macos")]

use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use porthole_core::PortholeError;

/// Returns the current cursor position in global screen coordinates.
/// Returns `Err` if the CG event machinery is unavailable (e.g., permission denied).
pub(crate) fn cursor_position() -> Result<(f64, f64), PortholeError> {
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
        PortholeError::new(
            porthole_core::ErrorCode::PermissionNeeded,
            "cursor_position: event source failed",
        )
    })?;
    let ev = CGEvent::new(src).map_err(|_| {
        PortholeError::new(
            porthole_core::ErrorCode::PermissionNeeded,
            "cursor_position: event create failed",
        )
    })?;
    let loc = ev.location();
    Ok((loc.x, loc.y))
}
