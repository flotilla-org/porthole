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
}

pub async fn focus(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "focus: surface has no pid")
    })? as i32;

    // Activate the owning app via NSRunningApplication.
    activate_app(pid)?;

    // Raise the specific window (best effort). If we can't locate it, continue —
    // activating the app is usually enough.
    let _ = with_first_window_for_pid(pid, |win| {
        unsafe {
            let action = CFString::new("AXRaise");
            let _ = AXUIElementPerformAction(win, action.as_concrete_TypeRef() as CFStringRef);
        }
        Ok(())
    });
    Ok(())
}

pub async fn close(surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "close: surface has no pid")
    })? as i32;

    let via_close_button = with_first_window_for_pid(pid, |win| unsafe {
        let close_button_attr = CFString::new("AXCloseButton");
        let mut button_ptr: *const std::ffi::c_void = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(
            win,
            close_button_attr.as_concrete_TypeRef() as CFStringRef,
            &mut button_ptr,
        );
        if err == K_AXERROR_SUCCESS && !button_ptr.is_null() {
            let press = CFString::new("AXPress");
            let _ =
                AXUIElementPerformAction(button_ptr as AXUIElementRef, press.as_concrete_TypeRef() as CFStringRef);
            CFRelease(button_ptr);
            Ok(true)
        } else {
            Ok(false)
        }
    });
    if matches!(via_close_button, Ok(true)) {
        return Ok(());
    }

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
    Ok(())
}

pub async fn window_bounds(surface: &SurfaceInfo) -> Result<Rect, PortholeError> {
    use crate::enumerate::list_windows;
    let pid = surface.pid.ok_or_else(|| {
        PortholeError::new(ErrorCode::CapabilityMissing, "window_bounds: surface has no pid")
    })? as i32;
    let windows = list_windows()?;
    let hit = windows.iter().find(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title));
    match hit {
        Some(_w) => {
            // CGWindowList doesn't give us bounds in our `WindowRecord`. For v0 we
            // read them from AX below.
            bounds_from_ax(pid, surface.title.as_deref())
        }
        None => Err(PortholeError::new(ErrorCode::SurfaceDead, "window_bounds: no matching window")),
    }
}

fn bounds_from_ax(pid: i32, _title: Option<&str>) -> Result<Rect, PortholeError> {
    with_first_window_for_pid(pid, |win| unsafe {
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
    })
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

fn activate_app(pid: i32) -> Result<(), PortholeError> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    use objc2::rc::Retained;

    unsafe {
        let app: Option<Retained<NSRunningApplication>> =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        match app {
            Some(a) => {
                let opts = NSApplicationActivationOptions::empty();
                NSRunningApplication::activateWithOptions(&*a, opts);
                Ok(())
            }
            None => Err(PortholeError::new(ErrorCode::SurfaceDead, "no running app for pid")),
        }
    }
}
