#![cfg(target_os = "macos")]

use core_foundation::array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef};
use porthole_core::{ErrorCode, PortholeError, adapter::Rect, surface::SurfaceInfo};

use crate::{
    MacOsAdapter,
    ax::{AXValueGetValue, AxElement, AxElementRef},
    permissions::ensure_accessibility_granted,
};

// AXValue type tags for CGPoint and CGSize.
const K_AX_VALUE_CG_POINT_TYPE: i32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: i32 = 2;

pub async fn focus(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "focus: surface has no pid"))? as i32;

    // Activate the owning app via NSRunningApplication.
    activate_app(pid)?;

    // Raise the specific window. If cg_window_id is present we must resolve it
    // exactly — a miss means the window is gone (SurfaceDead). Without an id we
    // fall back to AXWindows[0] (pre-slice-A behavior).
    let raise = |win: AxElementRef| -> Result<(), PortholeError> {
        // SAFETY: win is borrowed from the AXWindows array held alive by the
        // caller (with_first_window_for_pid / with_ax_window_by_cg_id).
        unsafe { crate::ax::perform_action_borrowed(win, "AXRaise") };
        Ok(())
    };
    if let Some(cg_id) = surface.cg_window_id {
        with_ax_window_by_cg_id(pid, cg_id, raise)?;
    } else {
        // Best-effort: failing to raise AXWindows[0] is non-fatal.
        let _ = with_first_window_for_pid(pid, raise);
    }
    Ok(())
}

pub async fn close(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    use tokio::time::sleep;

    use crate::enumerate::list_windows;

    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "close: surface has no pid"))? as i32;

    let press_close_button = |win: AxElementRef| -> Result<bool, PortholeError> {
        // SAFETY: win is borrowed from the AXWindows array held alive by the caller.
        let button_ptr = unsafe { crate::ax::copy_attribute_borrowed(win, "AXCloseButton") };
        if let Some(btn) = button_ptr {
            // btn is a retained AXUIElementRef for the close button.
            // Wrap it in AxElement so it's released on drop.
            if let Some(e) = unsafe { AxElement::from_retained(btn) } {
                e.perform_action("AXPress");
            }
            Ok(true)
        } else {
            Ok(false)
        }
    };

    let via_close_button = if let Some(cg_id) = surface.cg_window_id {
        // Fail closed: if the window is already gone this returns SurfaceDead.
        with_ax_window_by_cg_id(pid, cg_id, press_close_button)?
    } else {
        with_first_window_for_pid(pid, press_close_button).unwrap_or(false)
    };

    if !via_close_button {
        // Fallback: focus + Cmd+W via input path.
        // accessibility was already preflighted at the top of close().
        focus(adapter, surface).await?;
        let src = core_graphics::event_source::CGEventSource::new(core_graphics::event_source::CGEventSourceStateID::HIDSystemState)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "close fallback: event source failed"))?;
        let code_w: u16 = 0x0D;
        let flags = core_graphics::event::CGEventFlags::CGEventFlagCommand;
        let down = core_graphics::event::CGEvent::new_keyboard_event(src.clone(), code_w, true)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "close fallback: down event failed"))?;
        down.set_flags(flags);
        down.post(core_graphics::event::CGEventTapLocation::HID);
        let up = core_graphics::event::CGEvent::new_keyboard_event(src, code_w, false)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "close fallback: up event failed"))?;
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
            windows
                .iter()
                .any(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title))
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
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "window_bounds: surface has no pid"))? as i32;
    let windows = list_windows()?;
    let hit = if let Some(cg_id) = surface.cg_window_id {
        windows.iter().find(|w| w.cg_window_id == cg_id)
    } else {
        windows
            .iter()
            .find(|w| w.owner_pid == pid && (surface.title.is_none() || w.title == surface.title))
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
    let read_bounds = |win: AxElementRef| -> Result<Rect, PortholeError> {
        // SAFETY: win is borrowed from the AXWindows array held alive by the caller.
        let pos_ptr = unsafe { crate::ax::copy_attribute_borrowed(win, "AXPosition") };
        let size_ptr = unsafe { crate::ax::copy_attribute_borrowed(win, "AXSize") };
        let mut rect = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        if let Some(p) = pos_ptr {
            let mut pt = core_graphics::geometry::CGPoint { x: 0.0, y: 0.0 };
            unsafe { AXValueGetValue(p, K_AX_VALUE_CG_POINT_TYPE, &raw mut pt as *mut std::ffi::c_void) };
            rect.x = pt.x;
            rect.y = pt.y;
            unsafe { crate::ax::cf_release(p) };
        }
        if let Some(s) = size_ptr {
            let mut sz = core_graphics::geometry::CGSize { width: 0.0, height: 0.0 };
            unsafe { AXValueGetValue(s, K_AX_VALUE_CG_SIZE_TYPE, &raw mut sz as *mut std::ffi::c_void) };
            rect.w = sz.width;
            rect.h = sz.height;
            unsafe { crate::ax::cf_release(s) };
        }
        Ok(rect)
    };
    if let Some(cg_id) = cg_window_id {
        with_ax_window_by_cg_id(pid, cg_id, read_bounds)
    } else {
        with_first_window_for_pid(pid, read_bounds)
    }
}

/// Run `op` against the first AX window of the given pid. The `AxElementRef`
/// passed to `op` is borrowed from the AXWindows array (held alive by this
/// function until `op` returns) and must not be stored beyond the closure.
fn with_first_window_for_pid<F, R>(pid: i32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AxElementRef) -> Result<R, PortholeError>,
{
    let app = AxElement::for_application(pid)
        .ok_or_else(|| PortholeError::new(ErrorCode::SystemPermissionNeeded, "AXUIElementCreateApplication returned null"))?;
    let windows_ptr = app
        .copy_attribute_raw("AXWindows")
        .ok_or_else(|| PortholeError::new(ErrorCode::SystemPermissionNeeded, "AXWindows read failed"))?;
    // windows_ptr is a retained CFArrayRef; we hold it alive until after the closure.
    let arr = windows_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };
    let result = if count == 0 {
        Err(PortholeError::new(ErrorCode::SurfaceDead, "no AX windows found"))
    } else {
        let raw = unsafe { CFArrayGetValueAtIndex(arr, 0) } as AxElementRef;
        op(raw)
    };
    // Release the array we copied.
    unsafe { crate::ax::cf_release(windows_ptr) };
    result
}

/// Run `op` against the AX window whose CGWindowID matches `target`. Uses the
/// `_AXUIElementGetWindow` private API (widely used in macOS automation tooling;
/// stable across macOS versions since 10.9). Returns `SurfaceDead` if no AX
/// window matches — the window was closed externally or the id is stale.
/// **Does not fall back** to AXWindows[0]; call sites that have no CGWindowID
/// should use `with_first_window_for_pid` directly.
///
/// The `AxElementRef` passed to `op` is borrowed from the AXWindows array
/// (held alive by this function until `op` returns) and must not be stored
/// beyond the closure.
pub(crate) fn with_ax_window_by_cg_id<F, R>(pid: i32, target: u32, op: F) -> Result<R, PortholeError>
where
    F: FnOnce(AxElementRef) -> Result<R, PortholeError>,
{
    let app = AxElement::for_application(pid)
        .ok_or_else(|| PortholeError::new(ErrorCode::SystemPermissionNeeded, "AXUIElementCreateApplication returned null"))?;
    let windows_ptr = app
        .copy_attribute_raw("AXWindows")
        .ok_or_else(|| PortholeError::new(ErrorCode::SystemPermissionNeeded, "AXWindows read failed"))?;
    let arr = windows_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };

    let mut matched: Option<AxElementRef> = None;
    for i in 0..count {
        let raw = unsafe { CFArrayGetValueAtIndex(arr, i) } as AxElementRef;
        // Use the borrowed helper — does not consume or retain raw.
        let cg = unsafe { crate::ax::ax_get_window_id_borrowed(raw) };
        if cg == Some(target) {
            matched = Some(raw);
            break;
        }
    }

    let result = match matched {
        Some(raw) => op(raw),
        None => Err(PortholeError::new(
            ErrorCode::SurfaceDead,
            format!("window with cg_window_id {target} no longer exists for pid {pid}"),
        )),
    };
    // Release the array we copied.
    unsafe { crate::ax::cf_release(windows_ptr) };
    result
}

fn activate_app(pid: i32) -> Result<(), PortholeError> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

    unsafe {
        let app: Option<Retained<NSRunningApplication>> = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
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
