#![cfg(target_os = "macos")]

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use porthole_core::input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec};
use porthole_core::surface::SurfaceInfo;
use porthole_core::{ErrorCode, PortholeError};

use crate::close_focus;
use crate::key_codes::key_code;
use crate::MacOsAdapter;
use crate::permissions::ensure_accessibility_granted;

fn event_source() -> Result<CGEventSource, PortholeError> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "failed to create CGEventSource"))
}

fn flags_for(modifiers: &[Modifier]) -> CGEventFlags {
    let mut flags = CGEventFlags::empty();
    for m in modifiers {
        flags |= match m {
            Modifier::Cmd => CGEventFlags::CGEventFlagCommand,
            Modifier::Ctrl => CGEventFlags::CGEventFlagControl,
            Modifier::Alt => CGEventFlags::CGEventFlagAlternate,
            Modifier::Shift => CGEventFlags::CGEventFlagShift,
        };
    }
    flags
}

pub async fn key(adapter: &MacOsAdapter, surface: &SurfaceInfo, events: &[KeyEvent]) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    close_focus::focus(adapter, surface).await?;
    let source = event_source()?;
    for ev in events {
        let code = key_code(&ev.key).ok_or_else(|| {
            PortholeError::new(ErrorCode::UnknownKey, format!("no keycode for '{}'", ev.key))
        })?;
        let flags = flags_for(&ev.modifiers);

        let down = CGEvent::new_keyboard_event(source.clone(), code, true)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "key down event create failed"))?;
        down.set_flags(flags);
        down.post(CGEventTapLocation::HID);

        let up = CGEvent::new_keyboard_event(source.clone(), code, false)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "key up event create failed"))?;
        up.set_flags(flags);
        up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

pub async fn text(adapter: &MacOsAdapter, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    close_focus::focus(adapter, surface).await?;
    let source = event_source()?;

    let units: Vec<u16> = text.encode_utf16().collect();
    let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "text event create failed"))?;
    down.set_string_from_utf16_unchecked(&units);
    down.post(CGEventTapLocation::HID);

    let up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "text up event create failed"))?;
    up.set_string_from_utf16_unchecked(&units);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

pub async fn click(adapter: &MacOsAdapter, surface: &SurfaceInfo, spec: &ClickSpec) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    close_focus::focus(adapter, surface).await?;
    let source = event_source()?;
    let flags = flags_for(&spec.modifiers);
    let (down_ty, up_ty, button) = match spec.button {
        ClickButton::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp, CGMouseButton::Left),
        ClickButton::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp, CGMouseButton::Right),
        ClickButton::Middle => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGMouseButton::Center),
    };
    let pos = CGPoint::new(screen_x, screen_y);
    for n in 1..=spec.count as i64 {
        let down = CGEvent::new_mouse_event(source.clone(), down_ty, pos, button)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "mouse down create failed"))?;
        down.set_flags(flags);
        down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, n);
        down.post(CGEventTapLocation::HID);

        let up = CGEvent::new_mouse_event(source.clone(), up_ty, pos, button)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "mouse up create failed"))?;
        up.set_flags(flags);
        up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, n);
        up.post(CGEventTapLocation::HID);
    }
    Ok(())
}

pub async fn scroll(adapter: &MacOsAdapter, surface: &SurfaceInfo, spec: &ScrollSpec) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    // Scroll events on macOS are positioned at the mouse cursor, so we move
    // the cursor to the window-local point first. This is a visible side
    // effect; acceptable for v0.x.
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    close_focus::focus(adapter, surface).await?;
    let source = event_source()?;

    // Move cursor.
    let move_ev = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::MouseMoved,
        CGPoint::new(screen_x, screen_y),
        CGMouseButton::Left,
    )
    .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "cursor move failed"))?;
    move_ev.post(CGEventTapLocation::HID);

    let scroll_ev = CGEvent::new_scroll_event(
        source,
        ScrollEventUnit::LINE,
        2, // axis count: vertical + horizontal
        spec.delta_y as i32,
        spec.delta_x as i32,
        0,
    )
    .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "scroll event create failed"))?;
    scroll_ev.post(CGEventTapLocation::HID);
    Ok(())
}

/// Converts window-local logical points to screen-global logical points using
/// the current window bounds from AX, and validates that the point lies within
/// the window bounds (with a 1-point tolerance for rounding on edges).
async fn window_to_screen(surface: &SurfaceInfo, x: f64, y: f64) -> Result<(f64, f64), PortholeError> {
    let bounds = crate::close_focus::window_bounds(surface).await?;
    const TOLERANCE: f64 = 1.0;
    if x < -TOLERANCE
        || x > bounds.w + TOLERANCE
        || y < -TOLERANCE
        || y > bounds.h + TOLERANCE
    {
        return Err(PortholeError::new(
            ErrorCode::InvalidCoordinate,
            format!(
                "coordinate ({x}, {y}) is outside window bounds (w={w}, h={h})",
                w = bounds.w,
                h = bounds.h,
            ),
        ));
    }
    Ok((bounds.x + x, bounds.y + y))
}
