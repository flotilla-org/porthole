#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use porthole_core::adapter::Rect;
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

// AX constants and function bindings — minimal subset we need.
type AXUIElementRef = *const std::ffi::c_void;
type AXError = i32;
const K_AXERROR_SUCCESS: AXError = 0;

unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut *const std::ffi::c_void,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    fn CFRelease(ptr: *const std::ffi::c_void);

    // Private API: widely used in macOS automation tooling (e.g. Hammerspoon,
    // Accessibility Inspector). Stable across macOS versions since 10.9.
    fn _AXUIElementGetWindow(element: AXUIElementRef, out: *mut u32) -> AXError;
}

pub async fn focus(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "focus: surface has no pid")
    })? as i32;

    // Activate the owning app via NSRunningApplication.
    activate_app(pid)?;

    // Raise the specific window (best effort). If we can't locate it, continue —
    // activating the app is usually enough.
    let raise = |win: AXUIElementRef| -> Result<(), PortholeError> {
        unsafe {
            let action = CFString::new("AXRaise");
            let _ = AXUIElementPerformAction(win, action.as_concrete_TypeRef() as CFStringRef);
        }
        Ok(())
    };
    let _ = if let Some(cg_id) = surface.cg_window_id {
        with_ax_window_by_cg_id(pid, cg_id, raise)
    } else {
        with_first_window_for_pid(pid, raise)
    };
    Ok(())
}

pub async fn close(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    use crate::enumerate::list_windows;
    use tokio::time::sleep;

    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "close: surface has no pid")
    })? as i32;

    let press_close_button = |win: AXUIElementRef| -> Result<bool, PortholeError> {
        unsafe {
            let close_button_attr = CFString::new("AXCloseButton");
            let mut button_ptr: *const std::ffi::c_void = std::ptr::null();
            let err = AXUIElementCopyAttributeValue(
                win,
                close_button_attr.as_concrete_TypeRef() as CFStringRef,
                &mut button_ptr,
            );
            if err == K_AXERROR_SUCCESS && !button_ptr.is_null() {
                let press = CFString::new("AXPress");
                let _ = AXUIElementPerformAction(
                    button_ptr as AXUIElementRef,
                    press.as_concrete_TypeRef() as CFStringRef,
                );
                CFRelease(button_ptr);
                Ok(true)
            } else {
                Ok(false)
            }
        }
    };

    let via_close_button = if let Some(cg_id) = surface.cg_window_id {
        with_ax_window_by_cg_id(pid, cg_id, press_close_button)
    } else {
        with_first_window_for_pid(pid, press_close_button)
    };

    if !matches!(via_close_button, Ok(true)) {
        // Fallback: focus + Cmd+W via input path.
        focus(surface).await?;
        let src = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: event source failed"))?;
        let code_w: u16 = 0x0D;
        let flags = core_graphics::event::CGEventFlags::CGEventFlagCommand;
        let down = core_graphics::event::CGEvent::new_keyboard_event(src.clone(), code_w, true)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: down event failed"))?;
        down.set_flags(flags);
        down.post(core_graphics::event::CGEventTapLocation::HID);
        let up = core_graphics::event::CGEvent::new_keyboard_event(src, code_w, false)
            .map_err(|_| PortholeError::new(ErrorCode::PermissionNeeded, "close fallback: up event failed"))?;
        up.set_flags(flags);
        up.post(core_graphics::event::CGEventTapLocation::HID);
    }

    // Poll up to 6 times with 100ms spacing (600ms total) to verify the window
    // actually closed. A save dialog or veto can leave it open.
    for _ in 0..6 {
        sleep(std::time::Duration::from_millis(100)).await;
        let windows = list_windows()?;
        let still_present = if let Some(cg_id) = surface.cg_window_id {
            windows.iter().any(|w| w.cg_window_id == cg_id)
        } else {
            windows.iter().any(|w| {
                w.owner_pid == pid
                    && (surface.title.is_none() || w.title == surface.title)
            })
        };
        if !still_present {
            return Ok(());
        }
    }

    Err(PortholeError::new(
        ErrorCode::CloseFailed,
        "window did not close (possibly a save dialog or veto); handle remains alive",
    ))
}

pub async fn window_bounds(surface: &SurfaceInfo) -> Result<Rect, PortholeError> {
    use crate::enumerate::list_windows;
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "window_bounds: surface has no pid")
    })? as i32;
    let windows = list_windows()?;
    let hit = if let Some(cg_id) = surface.cg_window_id {
        windows.iter().find(|w| w.cg_window_id == cg_id)
    } else {
        windows.iter().find(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title))
    };
    match hit {
        Some(_w) => {
            // CGWindowList doesn't give us bounds in our `WindowRecord`. For v0 we
            // read them from AX below.
            bounds_from_ax(pid, surface.cg_window_id)
        }
        None => Err(PortholeError::new(ErrorCode::SurfaceDead, "window_bounds: no matching window")),
    }
}

fn bounds_from_ax(pid: i32, cg_window_id: Option<u32>) -> Result<Rect, PortholeError> {
    let read_bounds = |win: AXUIElementRef| -> Result<Rect, PortholeError> {
        unsafe {
            let pos_attr = CFString::new("AXPosition");
            let size_attr = CFString::new("AXSize");
            let mut pos_ptr: *const std::ffi::c_void = std::ptr::null();
            let mut size_ptr: *const std::ffi::c_void = std::ptr::null();
            let _ = AXUIElementCopyAttributeValue(win, pos_attr.as_concrete_TypeRef() as CFStringRef, &mut pos_ptr);
            let _ = AXUIElementCopyAttributeValue(win, size_attr.as_concrete_TypeRef() as CFStringRef, &mut size_ptr);
            let mut rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            if !pos_ptr.is_null() {
                let mut pt = core_graphics::geometry::CGPoint { x: 0.0, y: 0.0 };
                ax_value_to_point(pos_ptr, &mut pt);
                rect.x = pt.x;
                rect.y = pt.y;
                CFRelease(pos_ptr);
            }
            if !size_ptr.is_null() {
                let mut sz = core_graphics::geometry::CGSize { width: 0.0, height: 0.0 };
                ax_value_to_size(size_ptr, &mut sz);
                rect.w = sz.width;
                rect.h = sz.height;
                CFRelease(size_ptr);
            }
            Ok(rect)
        }
    };
    if let Some(cg_id) = cg_window_id {
        with_ax_window_by_cg_id(pid, cg_id, read_bounds)
    } else {
        with_first_window_for_pid(pid, read_bounds)
    }
}

const K_AX_VALUE_CG_POINT_TYPE: i32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: i32 = 2;

unsafe extern "C" {
    fn AXValueGetValue(value: *const std::ffi::c_void, the_type: i32, value_ptr: *mut std::ffi::c_void) -> u8;
}

unsafe fn ax_value_to_point(v: *const std::ffi::c_void, out: *mut core_graphics::geometry::CGPoint) {
    unsafe {
        AXValueGetValue(v, K_AX_VALUE_CG_POINT_TYPE, out as *mut std::ffi::c_void);
    }
}

unsafe fn ax_value_to_size(v: *const std::ffi::c_void, out: *mut core_graphics::geometry::CGSize) {
    unsafe {
        AXValueGetValue(v, K_AX_VALUE_CG_SIZE_TYPE, out as *mut std::ffi::c_void);
    }
}

/// Run `op` against the first AX window of the given pid, handling retain/release
/// for both the application element and the window-list array. The AXUIElementRef
/// passed to `op` is borrowed from the array and is valid only for the duration
/// of the call — `op` must not return it or retain pointers into its children
/// without explicit CFRetain.
fn with_first_window_for_pid<F, R>(pid: i32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AXUIElementRef) -> Result<R, PortholeError>,
{
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXUIElementCreateApplication returned null"));
        }
        let windows_attr = CFString::new("AXWindows");
        let mut windows_ptr: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            windows_attr.as_concrete_TypeRef() as CFStringRef,
            &mut windows_ptr,
        );
        if err != K_AXERROR_SUCCESS || windows_ptr.is_null() {
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXWindows read failed"));
        }
        let arr = windows_ptr as core_foundation::array::CFArrayRef;
        let count = core_foundation::array::CFArrayGetCount(arr);
        if count == 0 {
            CFRelease(windows_ptr);
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::SurfaceDead, "no AX windows found"));
        }
        let win = core_foundation::array::CFArrayGetValueAtIndex(arr, 0) as AXUIElementRef;
        // `win` lives inside the array; safe to use until we release `windows_ptr`.
        let result = op(win);
        CFRelease(windows_ptr);
        CFRelease(app);
        result
    }
}

/// Run `op` against the AX window whose CGWindowID matches `target`. Uses the
/// `_AXUIElementGetWindow` private API (widely used in macOS automation tooling;
/// stable across macOS versions since 10.9). Falls back to `AXWindows[0]` with a
/// warning trace if the lookup fails — surfaces created before slice-A may not
/// have `cg_window_id` populated.
fn with_ax_window_by_cg_id<F, R>(pid: i32, target: u32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AXUIElementRef) -> Result<R, PortholeError>,
{
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXUIElementCreateApplication returned null"));
        }
        let windows_attr = CFString::new("AXWindows");
        let mut windows_ptr: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            app,
            windows_attr.as_concrete_TypeRef() as CFStringRef,
            &mut windows_ptr,
        );
        if err != K_AXERROR_SUCCESS || windows_ptr.is_null() {
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::PermissionNeeded, "AXWindows read failed"));
        }
        let arr = windows_ptr as core_foundation::array::CFArrayRef;
        let count = core_foundation::array::CFArrayGetCount(arr);
        if count == 0 {
            CFRelease(windows_ptr);
            CFRelease(app);
            return Err(PortholeError::new(ErrorCode::SurfaceDead, "no AX windows found"));
        }

        // Iterate windows looking for a CGWindowID match.
        let mut matched_idx: Option<isize> = None;
        for i in 0..count {
            let win = core_foundation::array::CFArrayGetValueAtIndex(arr, i) as AXUIElementRef;
            let mut wid: u32 = 0;
            let ax_err = _AXUIElementGetWindow(win, &mut wid);
            if ax_err == K_AXERROR_SUCCESS && wid == target {
                matched_idx = Some(i);
                break;
            }
        }

        let idx = matched_idx.unwrap_or_else(|| {
            tracing::warn!(
                target: "porthole_adapter_macos",
                pid,
                cg_window_id = target,
                "with_ax_window_by_cg_id: no AX window matched CGWindowID; falling back to AXWindows[0]"
            );
            0
        });

        let win = core_foundation::array::CFArrayGetValueAtIndex(arr, idx) as AXUIElementRef;
        let result = op(win);
        CFRelease(windows_ptr);
        CFRelease(app);
        result
    }
}

fn activate_app(pid: i32) -> Result<(), PortholeError> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use objc2::rc::Retained;

    unsafe {
        let app: Option<Retained<NSRunningApplication>> =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        match app {
            Some(a) => {
                let opts = NSApplicationActivationOptions::empty();
                NSRunningApplication::activateWithOptions(&a, opts);
                Ok(())
            }
            None => Err(PortholeError::new(ErrorCode::SurfaceDead, "no running app for pid")),
        }
    }
}
