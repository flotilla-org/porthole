#![cfg(target_os = "macos")]

use porthole_core::surface::{SurfaceId, SurfaceInfo, SurfaceKind, SurfaceState};
use porthole_core::PortholeError;

/// Enumerates all windows (including off-screen, minimized, and other-Space)
/// and returns a fresh SurfaceInfo if a window with the given
/// (pid, cg_window_id) exists.
///
/// Unlike `list_windows()` which uses `kCGWindowListOptionOnScreenOnly`, this
/// uses a broader option set so tracked handles remain valid through hide /
/// minimize / Space-switch cycles.
pub async fn window_alive(pid: u32, cg_window_id: u32) -> Result<Option<SurfaceInfo>, PortholeError> {
    use core_foundation::base::TCFType;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionAll, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
        kCGWindowOwnerPID,
    };
    use core_foundation::base::CFType;

    let opts = kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements;
    let arr = match copy_window_info(opts, kCGNullWindowID) {
        Some(a) => a,
        None => return Ok(None),
    };

    let count = arr.len();
    for i in 0..count {
        // Safety: i is within [0, count), and the elements are CFDictionary<CFString, CFType>.
        let raw_ptr: *const std::ffi::c_void = unsafe { *arr.get_unchecked(i) };
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(raw_ptr as CFDictionaryRef) };

        let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
        let owner_pid = dict
            .find(&owner_pid_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i32())
            .unwrap_or(0);
        if owner_pid as u32 != pid {
            continue;
        }

        let window_number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
        let this_cg = dict
            .find(&window_number_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i32())
            .map(|n| n as u32)
            .unwrap_or(0);
        if this_cg != cg_window_id {
            continue;
        }

        let window_name_key = unsafe { CFString::wrap_under_get_rule(kCGWindowName) };
        let title = dict
            .find(&window_name_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        let owner_name_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
        let app_name = dict
            .find(&owner_name_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        let info = SurfaceInfo {
            id: SurfaceId::new(),
            kind: SurfaceKind::Window,
            state: SurfaceState::Alive,
            title,
            app_name,
            pid: Some(pid),
            parent_surface_id: None,
            cg_window_id: Some(cg_window_id),
        };
        return Ok(Some(info));
    }
    Ok(None)
}
