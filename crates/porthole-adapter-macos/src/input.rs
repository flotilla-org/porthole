#![cfg(target_os = "macos")]

use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField, ScrollEventUnit},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use porthole_core::{
    ErrorCode, PortholeError,
    input::{ButtonSpec, ClickButton, ClickSpec, KeyEvent, KeyStrokeSpec, Modifier, PointerMoveSpec, PressAction, ScrollSpec},
    surface::{SurfaceId, SurfaceInfo},
};

use crate::{
    MacOsAdapter, close_focus,
    key_codes::{key_code, modifier_flag},
    permissions::ensure_accessibility_granted,
};

/// What `key_stroke` and `button` have left down. A key press is bound to
/// the keycode and flags its down posted, so its up and repeats reuse them
/// even if the caller's key name changes meanwhile; a button is bound to the
/// screen point and flags of its down, updated by drags, so its up lands
/// where the pointer is.
#[derive(Debug, Default)]
pub struct Held {
    keys: std::collections::HashMap<(SurfaceId, u64), HeldKey>,
    buttons: std::collections::HashMap<(SurfaceId, ClickButton), HeldButton>,
}

#[derive(Debug, Clone, Copy)]
struct HeldKey {
    code: u16,
    flags: CGEventFlags,
    pid: i32,
}

#[derive(Debug, Clone, Copy)]
struct HeldButton {
    x: f64,
    y: f64,
    flags: CGEventFlags,
}

/// A key transition for `code` carrying `flags`. Modifier keycodes become
/// flags-changed events with their own flag set on the way down and cleared
/// on the way up, which is what a physical modifier key produces.
fn keyboard_event(source: &CGEventSource, code: u16, down: bool, flags: CGEventFlags) -> Result<CGEvent, PortholeError> {
    let event = CGEvent::new_keyboard_event(source.clone(), code, down)
        .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "key event create failed"))?;
    match modifier_flag(code) {
        Some(own) => {
            event.set_type(CGEventType::FlagsChanged);
            event.set_flags(if down { flags | own } else { flags & !own });
        }
        None => event.set_flags(flags),
    }
    Ok(event)
}

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
        keyboard_event(&source, code, true, flags)?.post(CGEventTapLocation::HID);
        keyboard_event(&source, code, false, flags)?.post(CGEventTapLocation::HID);
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
/// Multi-window caveat: `post_to_pid` targets a *process*, not a window.
/// Apps with multiple windows (terminals running multiple shells, browsers,
/// editors with split panes) route keyboard input to whichever window the
/// app considers its key window — the OS doesn't expose per-window keyboard
/// delivery for synthetic events (unlike mouse events, which can carry a
/// window-id event field). Callers targeting a specific window of a
/// multi-window app must call `focus()` first so the app pins that window
/// as key. Peekaboo's `BackgroundInputDriver` lives with the same
/// constraint.
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

pub async fn pointer_move(adapter: &MacOsAdapter, surface: &SurfaceInfo, spec: &PointerMoveSpec) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    // A button held through `button` turns motion into a drag of that
    // button, and the held point follows so its up lands here. Held state
    // is checked before focusing: a drag must not re-raise the window.
    let dragging = {
        let mut held = adapter.held.lock().expect("held state poisoned");
        let mut dragging = None;
        for ((id, button), at) in held.buttons.iter_mut() {
            if *id == surface.id {
                at.x = screen_x;
                at.y = screen_y;
                dragging.get_or_insert((*button, *at));
            }
        }
        dragging
    };
    if let Some((button, at)) = dragging {
        let (_, _, drag_ty, mouse_button) = button_types(button);
        return post_button(&event_source()?, drag_ty, mouse_button, at);
    }
    close_focus::focus(adapter, surface).await?;
    // The event source is created after the await: it is not `Send`.
    let source = event_source()?;
    // Motion-only: no button state change. CGMouseButton::Left is required by
    // the API but ignored for MouseMoved events.
    let move_ev = CGEvent::new_mouse_event(
        source,
        CGEventType::MouseMoved,
        CGPoint::new(screen_x, screen_y),
        CGMouseButton::Left,
    )
    .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "pointer move event create failed"))?;
    move_ev.post(CGEventTapLocation::HID);
    Ok(())
}

fn button_types(button: ClickButton) -> (CGEventType, CGEventType, CGEventType, CGMouseButton) {
    match button {
        ClickButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            CGEventType::LeftMouseDragged,
            CGMouseButton::Left,
        ),
        ClickButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            CGEventType::RightMouseDragged,
            CGMouseButton::Right,
        ),
        ClickButton::Middle => (
            CGEventType::OtherMouseDown,
            CGEventType::OtherMouseUp,
            CGEventType::OtherMouseDragged,
            CGMouseButton::Center,
        ),
    }
}

fn post_key(source: &CGEventSource, held: HeldKey, down: bool, repeat: bool) -> Result<(), PortholeError> {
    let event = keyboard_event(source, held.code, down, held.flags)?;
    if repeat {
        event.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, 1);
    }
    // Posted to the process like `text`, so a controller's keys reach the
    // surface it drives without contending for HID focus on every event;
    // the executor focuses the surface once when the controller is admitted.
    event.post_to_pid(held.pid);
    Ok(())
}

fn post_button(source: &CGEventSource, ty: CGEventType, button: CGMouseButton, at: HeldButton) -> Result<(), PortholeError> {
    let event = CGEvent::new_mouse_event(source.clone(), ty, CGPoint::new(at.x, at.y), button)
        .map_err(|_| PortholeError::new(ErrorCode::SystemPermissionNeeded, "mouse event create failed"))?;
    event.set_flags(at.flags);
    event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
    event.post(CGEventTapLocation::HID);
    Ok(())
}

/// One keyboard transition with press identity. A down binds the press to
/// the keycode and flags it posts; the up and repeats of that press reuse the
/// binding. An up for a press this adapter never saw down falls back to the
/// key name, so a controller whose executor restarted can still release.
pub async fn key_stroke(adapter: &MacOsAdapter, surface: &SurfaceInfo, spec: &KeyStrokeSpec) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let pid = surface
        .pid
        .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "key_stroke: surface has no pid"))? as i32;
    if !is_pid_alive(pid) {
        return Err(PortholeError::new(
            ErrorCode::SurfaceDead,
            format!("key_stroke: target process {pid} is not running"),
        ));
    }
    let resolve = || -> Result<HeldKey, PortholeError> {
        let code =
            key_code(&spec.key).ok_or_else(|| PortholeError::new(ErrorCode::UnknownKey, format!("no keycode for '{}'", spec.key)))?;
        Ok(HeldKey {
            code,
            flags: flags_for(&spec.modifiers),
            pid,
        })
    };
    let key = (surface.id.clone(), spec.press);
    let source = event_source()?;
    match spec.action {
        PressAction::Down => {
            let held = resolve()?;
            adapter.held.lock().expect("held state poisoned").keys.insert(key, held);
            post_key(&source, held, true, false)
        }
        PressAction::Repeat => {
            let held = match adapter.held.lock().expect("held state poisoned").keys.get(&key) {
                Some(h) => *h,
                None => resolve()?,
            };
            post_key(&source, held, true, true)
        }
        PressAction::Up => {
            let held = match adapter.held.lock().expect("held state poisoned").keys.remove(&key) {
                Some(h) => h,
                None => resolve()?,
            };
            post_key(&source, held, false, false)
        }
    }
}

/// One button transition at a window-local point. A down focuses the surface
/// (a click on a background window would only activate it) and records the
/// button as held at that point; motion while held is posted as a drag by
/// `pointer_move`; the up is posted where the pointer last was.
pub async fn button(adapter: &MacOsAdapter, surface: &SurfaceInfo, spec: &ButtonSpec) -> Result<(), PortholeError> {
    ensure_accessibility_granted(adapter)?;
    let (screen_x, screen_y) = window_to_screen(surface, spec.x, spec.y).await?;
    let (down_ty, up_ty, _, mouse_button) = button_types(spec.button);
    let key = (surface.id.clone(), spec.button);
    let at = HeldButton {
        x: screen_x,
        y: screen_y,
        flags: flags_for(&spec.modifiers),
    };
    match spec.action {
        PressAction::Down => {
            close_focus::focus(adapter, surface).await?;
            adapter.held.lock().expect("held state poisoned").buttons.insert(key, at);
            // The event source is created after the await: it is not `Send`.
            post_button(&event_source()?, down_ty, mouse_button, at)
        }
        PressAction::Up => {
            adapter.held.lock().expect("held state poisoned").buttons.remove(&key);
            post_button(&event_source()?, up_ty, mouse_button, at)
        }
        PressAction::Repeat => Err(PortholeError::new(ErrorCode::InvalidArgument, "a button transition is down or up")),
    }
}

/// Releases every key and button held on `surface`. Keys go up in the order
/// they were pressed is not knowable here, so modifiers and plain keys are
/// released together with the flags their downs carried.
pub async fn release_held(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<(), PortholeError> {
    let (keys, buttons) = {
        let mut held = adapter.held.lock().expect("held state poisoned");
        let key_ids: Vec<(SurfaceId, u64)> = held.keys.keys().filter(|(id, _)| *id == surface.id).cloned().collect();
        let keys: Vec<HeldKey> = key_ids.iter().filter_map(|k| held.keys.remove(k)).collect();
        let button_ids: Vec<(SurfaceId, ClickButton)> = held.buttons.keys().filter(|(id, _)| *id == surface.id).cloned().collect();
        let buttons: Vec<(ClickButton, HeldButton)> = button_ids
            .iter()
            .filter_map(|k| held.buttons.remove(k).map(|at| (k.1, at)))
            .collect();
        (keys, buttons)
    };
    if keys.is_empty() && buttons.is_empty() {
        return Ok(());
    }
    ensure_accessibility_granted(adapter)?;
    let source = event_source()?;
    let mut first_error = None;
    for key in keys {
        if let Err(e) = post_key(&source, key, false, false) {
            first_error.get_or_insert(e);
        }
    }
    for (button, at) in buttons {
        let (_, up_ty, _, mouse_button) = button_types(button);
        if let Err(e) = post_button(&source, up_ty, mouse_button, at) {
            first_error.get_or_insert(e);
        }
    }
    first_error.map_or(Ok(()), Err)
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
