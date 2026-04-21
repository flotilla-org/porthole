#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use porthole_core::display::{DisplayId, DisplayInfo, Rect as DisplayRect};
use porthole_core::PortholeError;

pub async fn displays() -> Result<Vec<DisplayInfo>, PortholeError> {
    let ids = CGDisplay::active_displays().map_err(|e| {
        PortholeError::new(porthole_core::ErrorCode::CapabilityMissing, format!("active_displays failed: {e:?}"))
    })?;
    let main_id = CGDisplay::main().id;

    // Determine which display contains the cursor so we can set `focused`.
    let cursor = match crate::cursor::cursor_position() {
        Ok(pos) => Some(pos),
        Err(e) => {
            tracing::debug!("displays: could not obtain cursor position, focused will be false for all ({e})");
            None
        }
    };

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let display = CGDisplay::new(id);
        let bounds = display.bounds();
        let (pixels_w, pixels_h) = (display.pixels_wide(), display.pixels_high());
        let scale = if bounds.size.width > 0.0 { pixels_w as f64 / bounds.size.width } else { 1.0 };
        let focused = cursor.is_some_and(|(cx, cy)| {
            cx >= bounds.origin.x
                && cx < bounds.origin.x + bounds.size.width
                && cy >= bounds.origin.y
                && cy < bounds.origin.y + bounds.size.height
        });
        out.push(DisplayInfo {
            id: DisplayId::new(format!("disp_{id}")),
            bounds: DisplayRect {
                x: bounds.origin.x,
                y: bounds.origin.y,
                w: bounds.size.width,
                h: bounds.size.height,
            },
            scale,
            primary: id == main_id,
            focused,
        });
        let _ = pixels_h;
    }
    Ok(out)
}
