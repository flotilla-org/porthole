#![cfg(target_os = "macos")]

use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, ScrollEventUnit},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use porthole_core::{
    ErrorCode, PortholeError,
    input::{ClickButton, ClickSpec, KeyEvent, Modifier, ScrollSpec},
    surface::SurfaceInfo,
};

use crate::{MacOsAdapter, close_focus, key_codes::key_code, permissions::ensure_accessibility_granted};

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
        let code = key_code(&ev.key).ok_or_else(|| PortholeError::new(ErrorCode::UnknownKey, format!("no keycode for '{}'", ev.key)))?;
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

/// Inject `text` into `surface` as synthetic Unicode keystrokes, one codepoint
/// at a time, posted directly to the owning process via `CGEventPostToPid`.
///
/// Why per-character instead of one event with the full string as overlay:
/// `CGEventKeyboardSetUnicodeString` on a single keycode-0 event is racy on
/// macOS Tahoe — the OS can coalesce or silently drop the overlay when the
/// synthetic event source state accumulates over a session, which is what
/// the kitty-image-tests harness was hitting (first ~3 calls landed, then
/// silent drops). Per-character matches every well-known reference for
/// macOS text injection (Peekaboo's `TypeService`, Hammerspoon's
/// `hs.eventtap.keyStrokes`, WebDriver, PyAutoGUI).
///
/// Why `post_to_pid` instead of `post(CGEventTapLocation::HID)`:
/// HID-tap delivery depends on which window has OS-level keyboard focus at
/// the moment each event lands. That's racy when callers haven't pinned
/// focus first (and even when they have — focus transitions are async on
/// modern macOS). Posting to the target process's pid routes the event
/// directly to it without contending with whatever else the user is doing.
/// Side benefit: `text()` no longer steals focus from the user, so callers
/// that *do* want the window raised must call `focus()` explicitly first
/// (the standard order for terminal automation: focus → wait stable → text).
///
/// Explicit empty flags defend against modifier-state leakage from any
/// prior `key Ctrl-X`-style event whose up could have lagged.
pub async fn text(adapter: &MacOsAdapter, surface: &SurfaceInfo, text: &str) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "text: surface has no pid"))? as i32;

    // Confirm the target process is alive before we start posting events.
    // `CGEvent.post_to_pid` returns no status — once we hand events off they
    // either land or vanish, and we can't tell the difference. Checking once
    // up-front turns "process died yesterday, every char silently dropped"
    // into a clear SurfaceDead error. Matches Peekaboo's BackgroundInput-
    // Driver, which guards every action with the same pattern.
    if !is_pid_alive(pid) {
        return Err(PortholeError::new(
            ErrorCode::SurfaceDead,
            format!("text: target process {pid} is not running"),
        ));
    }

    let source = event_source()?;
    let empty_flags = CGEventFlags::empty();
    let mut buf = [0u16; 2];

    for ch in text.chars() {
        let units = ch.encode_utf16(&mut buf);
        let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "text down event create failed"))?;
        down.set_flags(empty_flags);
        down.set_string_from_utf16_unchecked(units);
        down.post_to_pid(pid);

        let up = CGEvent::new_keyboard_event(source.clone(), 0, false)
            .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "text up event create failed"))?;
        up.set_flags(empty_flags);
        up.post_to_pid(pid);
    }
    Ok(())
}

/// True if `pid` names a running process the caller could signal — used as a
/// lightweight pre-flight before pid-routed event posting. Treats `EPERM` as
/// "alive" since it implies the process exists but is privilege-protected;
/// matches Peekaboo's `isProcessAlive` so behavior is consistent across
/// macOS automation tools.
fn is_pid_alive(pid: i32) -> bool {
    // SAFETY: kill(pid, 0) sends no signal; it only tests existence and
    // signal permission. No preconditions on the pid value beyond it
    // being an int.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    // SAFETY: errno is a thread-local; reading it after a libc call is
    // standard.
    let errno = unsafe { *libc::__error() };
    errno == libc::EPERM
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
    if x < -TOLERANCE || x > bounds.w + TOLERANCE || y < -TOLERANCE || y > bounds.h + TOLERANCE {
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
