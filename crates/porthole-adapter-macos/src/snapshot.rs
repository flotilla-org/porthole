#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use porthole_core::{
    ErrorCode, PortholeError,
    display::{DisplayId, Rect},
    placement::GeometrySnapshot,
    surface::SurfaceInfo,
};

use crate::{MacOsAdapter, ax::AxElement, permissions::ensure_accessibility_granted};

/// Read the current global position + size of the tracked surface and
/// resolve which display it's on, returning display-local coordinates.
pub async fn snapshot_geometry(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<GeometrySnapshot, PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "snapshot_geometry: no pid"))? as i32;
    let cg = surface
        .cg_window_id
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "snapshot_geometry: no cg_window_id"))?;

    let (global_x, global_y, w, h) = crate::close_focus::with_ax_window_by_cg_id(pid, cg, |raw| {
        let (pos, size) = unsafe { AxElement::with_borrowed(raw, |elem| (elem.get_position(), elem.get_size())) };
        let (px, py) = pos.ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "AXPosition read failed"))?;
        let (sw, sh) = size.ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "AXSize read failed"))?;
        Ok((px, py, sw, sh))
    })?;

    // Resolve which display the window's center is on.
    let center_x = global_x + w / 2.0;
    let center_y = global_y + h / 2.0;
    let display_ids = CGDisplay::active_displays().unwrap_or_default();
    let (display_id, display_origin_x, display_origin_y) = display_ids
        .iter()
        .find_map(|id| {
            let display = CGDisplay::new(*id);
            let b = display.bounds();
            if center_x >= b.origin.x
                && center_x < b.origin.x + b.size.width
                && center_y >= b.origin.y
                && center_y < b.origin.y + b.size.height
            {
                Some((DisplayId::new(format!("disp_{id}")), b.origin.x, b.origin.y))
            } else {
                None
            }
        })
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "window center not on any active display"))?;

    Ok(GeometrySnapshot {
        display_id,
        display_local: Rect {
            x: global_x - display_origin_x,
            y: global_y - display_origin_y,
            w,
            h,
        },
    })
}
