#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use porthole_core::attention::{AttentionInfo, CursorPos};
use porthole_core::display::DisplayId;
use porthole_core::PortholeError;

/// Returns the CGWindowID of the frontmost on-screen window. Uses
/// `CGWindowListCreate` with `kCGWindowListOptionOnScreenOnly`; the first entry
/// in the returned list is front-to-back order (topmost window first).
pub fn frontmost_cg_window_id() -> Option<u32> {
    use core_graphics::window::{copy_window_info, kCGNullWindowID, kCGWindowListOptionOnScreenOnly};

    let arr = copy_window_info(kCGWindowListOptionOnScreenOnly, kCGNullWindowID)?;
    if arr.is_empty() {
        return None;
    }
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::kCGWindowNumber;

    let raw_ptr: *const std::ffi::c_void = unsafe { *arr.get_unchecked(0) };
    let dict: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_get_rule(raw_ptr as CFDictionaryRef) };
    let key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    dict.find(&key)
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_i32())
        .map(|n| n as u32)
}

pub async fn attention() -> Result<AttentionInfo, PortholeError> {
    let frontmost_name = frontmost_app_name();
    let cursor = crate::cursor::cursor_position()?;

    // Determine which display holds the cursor.
    let display_ids: Vec<u32> = CGDisplay::active_displays().unwrap_or_default();
    let mut cursor_display_id: Option<DisplayId> = None;
    for id in display_ids.iter() {
        let display = CGDisplay::new(*id);
        let b = display.bounds();
        if cursor.0 >= b.origin.x
            && cursor.0 < b.origin.x + b.size.width
            && cursor.1 >= b.origin.y
            && cursor.1 < b.origin.y + b.size.height
        {
            cursor_display_id = Some(DisplayId::new(format!("disp_{id}")));
            break;
        }
    }

    let focused_display_id = cursor_display_id.clone();

    Ok(AttentionInfo {
        focused_surface_id: None, // porthole-tracked focus matching is v0.1
        focused_app_name: frontmost_name,
        focused_display_id,
        cursor: CursorPos { x: cursor.0, y: cursor.1, display_id: cursor_display_id },
        recently_active_surface_ids: vec![],
    })
}

fn frontmost_app_name() -> Option<String> {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app: Option<objc2::rc::Retained<NSRunningApplication>> = workspace.frontmostApplication();
        app.and_then(|a| {
            let name = a.localizedName();
            name.map(|s| s.to_string())
        })
    }
}
