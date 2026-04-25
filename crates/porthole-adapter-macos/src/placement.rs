#![cfg(target_os = "macos")]

use porthole_core::display::Rect;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::ax::AxElement;
use crate::MacOsAdapter;
use crate::permissions::ensure_accessibility_granted;

/// Apply a global screen-coordinate rectangle to the tracked surface via
/// AX `AXPosition` + `AXSize` writes.
pub async fn place_surface(adapter: &MacOsAdapter, surface: &SurfaceInfo, rect: Rect) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| {
            PortholeError::new(ErrorCode::CapabilityMissing, "place_surface: no pid on surface")
        })?
        as i32;
    let cg = surface.cg_window_id.ok_or_else(|| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "place_surface: no cg_window_id on surface",
        )
    })?;

    crate::close_focus::with_ax_window_by_cg_id(pid, cg, |raw| {
        let (err_pos, err_size) = unsafe {
            AxElement::with_borrowed(raw, |elem| {
                (elem.set_position(rect.x, rect.y), elem.set_size(rect.w, rect.h))
            })
        };
        if err_pos != 0 || err_size != 0 {
            return Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                format!("AX refused position/size write: pos={err_pos} size={err_size}"),
            ));
        }
        Ok(())
    })
}
