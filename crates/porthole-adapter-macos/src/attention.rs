#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;
use core_graphics::event::CGEvent;
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use porthole_core::attention::{AttentionInfo, CursorPos};
use porthole_core::display::DisplayId;
use porthole_core::PortholeError;

pub async fn attention() -> Result<AttentionInfo, PortholeError> {
    let frontmost_bundle = frontmost_app_bundle();
    let cursor = cursor_position()?;

    // Determine which display holds the cursor.
    let display_ids: Vec<u32> = CGDisplay::active_displays().unwrap_or_default();
    let mut cursor_display_idx: Option<usize> = None;
    for (i, id) in display_ids.iter().enumerate() {
        let display = CGDisplay::new(*id);
        let b = display.bounds();
        if cursor.0 >= b.origin.x
            && cursor.0 < b.origin.x + b.size.width
            && cursor.1 >= b.origin.y
            && cursor.1 < b.origin.y + b.size.height
        {
            cursor_display_idx = Some(i);
            break;
        }
    }

    let focused_display_id = cursor_display_idx.map(|i| DisplayId::new(format!("disp_{}", display_ids[i])));

    Ok(AttentionInfo {
        focused_surface_id: None, // porthole-tracked focus matching is v0.1
        focused_app_bundle: frontmost_bundle,
        focused_display_id,
        cursor: CursorPos { x: cursor.0, y: cursor.1, display_id_index: cursor_display_idx },
        recently_active_surface_ids: vec![],
    })
}

fn frontmost_app_bundle() -> Option<String> {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app: Option<objc2::rc::Retained<NSRunningApplication>> = workspace.frontmostApplication();
        app.and_then(|a| {
            let bundle = a.bundleIdentifier();
            bundle.map(|s| s.to_string())
        })
    }
}

fn cursor_position() -> Result<(f64, f64), PortholeError> {
    // Use CGEvent::mouse_location via a temp event source.
    let src = CGEventSource::new(CGEventSourceStateID::HIDSystemState).map_err(|_| {
        PortholeError::new(porthole_core::ErrorCode::PermissionNeeded, "cursor_position: event source failed")
    })?;
    let ev = CGEvent::new(src).map_err(|_| {
        PortholeError::new(porthole_core::ErrorCode::PermissionNeeded, "cursor_position: event create failed")
    })?;
    let loc = ev.location();
    Ok((loc.x, loc.y))
}
