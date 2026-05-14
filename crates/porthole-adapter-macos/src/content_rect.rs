#![cfg(target_os = "macos")]

use core_foundation::{
    array::{CFArrayGetCount, CFArrayGetValueAtIndex, CFArrayRef},
    base::TCFType,
    string::{CFString, CFStringRef},
};
use porthole_core::{
    ErrorCode, PortholeError,
    content_rect::{ContentRectInfo, Descent},
    display::Rect,
    surface::SurfaceInfo,
};

use crate::{
    MacOsAdapter,
    ax::{AXValueGetValue, AxElementRef},
    close_focus,
    permissions::ensure_accessibility_granted,
};

// AXValue type tags — mirror constants used in close_focus.rs.
const K_AX_VALUE_CG_POINT_TYPE: i32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: i32 = 2;

pub async fn content_rect(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<ContentRectInfo, PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "content_rect: surface has no pid"))? as i32;
    let cg = surface
        .cg_window_id
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "content_rect: surface has no cg_window_id"))?;

    close_focus::with_ax_window_by_cg_id(pid, cg, read_content_rect)
}

fn read_content_rect(win: AxElementRef) -> Result<ContentRectInfo, PortholeError> {
    let outer = read_position(win)
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "content_rect: AXPosition read failed on window"))?;

    // Try AXContents first; fall back to AXChildren-by-area.
    if let Some(info) = try_axcontents_descent(win, outer)? {
        return Ok(info);
    }
    if let Some(info) = try_largest_child_descent(win, outer)? {
        return Ok(info);
    }
    Err(PortholeError::new(
        ErrorCode::ContentRectUnavailable,
        "content_rect: window exposes no usable content child via AXContents or AXChildren",
    ))
}

// Try reading AXContents. Returns Ok(Some(_)) when we found a usable first
// child, Ok(None) when the attribute is missing or empty (the largest-child
// fallback should run). Note: AXContents being *absent* vs. *present but
// empty* both surface as None here — both should fall through.
fn try_axcontents_descent(win: AxElementRef, outer: (f64, f64)) -> Result<Option<ContentRectInfo>, PortholeError> {
    let Some(arr_ptr) = (unsafe { crate::ax::copy_attribute_borrowed(win, "AXContents") }) else {
        return Ok(None);
    };
    let arr = arr_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };
    let result = if count == 0 {
        None
    } else {
        let child = unsafe { CFArrayGetValueAtIndex(arr, 0) } as AxElementRef;
        Some(extract_child_info(child, outer, Descent::Contents))
    };
    unsafe { crate::ax::cf_release(arr_ptr) };
    result.transpose()
}

fn try_largest_child_descent(win: AxElementRef, outer: (f64, f64)) -> Result<Option<ContentRectInfo>, PortholeError> {
    let Some(arr_ptr) = (unsafe { crate::ax::copy_attribute_borrowed(win, "AXChildren") }) else {
        return Ok(None);
    };
    let arr = arr_ptr as CFArrayRef;
    let count = unsafe { CFArrayGetCount(arr) };

    let mut best: Option<(AxElementRef, f64)> = None;
    // TODO(perf): each AXSize read crosses XPC; if harnesses report latency on
    // windows with many children, switch to AXUIElementCopyMultipleAttributeValues
    // for a single bulk fetch.
    for i in 0..count {
        let child = unsafe { CFArrayGetValueAtIndex(arr, i) } as AxElementRef;
        let Some((w, h)) = read_size(child) else { continue };
        let area = w * h;
        if !area.is_finite() || area <= 0.0 {
            continue;
        }
        match best {
            Some((_, best_area)) if area <= best_area => {}
            _ => best = Some((child, area)),
        }
    }

    let result = best.map(|(child, _)| extract_child_info(child, outer, Descent::LargestChild));
    unsafe { crate::ax::cf_release(arr_ptr) };
    result.transpose()
}

fn extract_child_info(child: AxElementRef, outer: (f64, f64), descent: Descent) -> Result<ContentRectInfo, PortholeError> {
    let (cx, cy) = read_position(child)
        .ok_or_else(|| PortholeError::new(ErrorCode::ContentRectUnavailable, "content_rect: AXPosition read failed on child"))?;
    let (cw, ch) = read_size(child)
        .ok_or_else(|| PortholeError::new(ErrorCode::ContentRectUnavailable, "content_rect: AXSize read failed on child"))?;
    // AXRole is debug-grade; if the read fails for whatever reason, fall back
    // to "unknown" rather than failing the whole call — the rect is the
    // payload.
    let ax_role = read_role(child).unwrap_or_else(|| "unknown".to_string());
    Ok(ContentRectInfo {
        rect: Rect {
            x: cx - outer.0,
            y: cy - outer.1,
            w: cw,
            h: ch,
        },
        ax_role,
        descent,
    })
}

fn read_position(el: AxElementRef) -> Option<(f64, f64)> {
    let ptr = unsafe { crate::ax::copy_attribute_borrowed(el, "AXPosition") }?;
    let mut pt = core_graphics::geometry::CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe { AXValueGetValue(ptr, K_AX_VALUE_CG_POINT_TYPE, &raw mut pt as *mut std::ffi::c_void) };
    unsafe { crate::ax::cf_release(ptr) };
    if ok != 0 { Some((pt.x, pt.y)) } else { None }
}

fn read_size(el: AxElementRef) -> Option<(f64, f64)> {
    let ptr = unsafe { crate::ax::copy_attribute_borrowed(el, "AXSize") }?;
    let mut sz = core_graphics::geometry::CGSize { width: 0.0, height: 0.0 };
    let ok = unsafe { AXValueGetValue(ptr, K_AX_VALUE_CG_SIZE_TYPE, &raw mut sz as *mut std::ffi::c_void) };
    unsafe { crate::ax::cf_release(ptr) };
    if ok != 0 { Some((sz.width, sz.height)) } else { None }
}

fn read_role(el: AxElementRef) -> Option<String> {
    let ptr = unsafe { crate::ax::copy_attribute_borrowed(el, "AXRole") }?;
    let s = unsafe { CFString::wrap_under_create_rule(ptr as CFStringRef) };
    Some(s.to_string())
}
