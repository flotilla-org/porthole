use porthole_core::{ErrorCode, PortholeError};

#[derive(Clone, Debug)]
pub struct WindowRecord {
    pub cg_window_id: u32,
    pub owner_pid: i32,
    pub title: Option<String>,
    pub app_bundle: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
        kCGWindowOwnerPID,
    };

    let opts = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let arr = match copy_window_info(opts, kCGNullWindowID) {
        Some(a) => a,
        None => {
            return Err(PortholeError::new(
                ErrorCode::CapabilityMissing,
                "CGWindowListCopyWindowInfo returned null",
            ));
        }
    };

    let count = arr.len();
    let mut out = Vec::with_capacity(count as usize);

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

        let window_number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
        let cg_window_id = dict
            .find(&window_number_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i32())
            .map(|n| n as u32)
            .unwrap_or(0);

        let window_name_key = unsafe { CFString::wrap_under_get_rule(kCGWindowName) };
        let title = dict
            .find(&window_name_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        let owner_name_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerName) };
        let app_bundle = dict
            .find(&owner_name_key)
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string());

        out.push(WindowRecord { cg_window_id, owner_pid, title, app_bundle });
    }

    if out.is_empty() {
        tracing::debug!("list_windows returned empty result");
    }
    Ok(out)
}

#[cfg(not(target_os = "macos"))]
pub fn list_windows() -> Result<Vec<WindowRecord>, PortholeError> {
    Err(PortholeError::new(
        ErrorCode::AdapterUnsupported,
        "macOS adapter not supported on this platform",
    ))
}

#[cfg(test)]
#[cfg(target_os = "macos")]
mod tests {
    use super::*;

    #[test]
    fn list_windows_runs_without_panicking() {
        // We do not assert a window count — CI sandboxes may have zero windows.
        let result = list_windows();
        assert!(result.is_ok(), "list_windows failed: {result:?}");
    }
}
