//! CGDirectDisplayID → backing scale factor lookup, via CoreGraphics only.
//!
//! `CGDisplayPixelsWide`/`CGDisplayPixelsHigh` on modern macOS return the
//! logical width/height of the active display mode (i.e., points), not the
//! backing pixel count — so they can't be used directly. But the *display
//! mode* object exposes both logical (`width`/`height`) and physical
//! (`pixel_width`/`pixel_height`) sizes, and the ratio is the backing scale.
//! That's enough; no NSScreen lookup needed.
//!
//! Why all-CG and not NSScreen: on macOS Tahoe (26 / Darwin 25),
//! `[NSScreen screens]` returns a Swift `[NSScreen]` array bridged to
//! NSArray via `_ContiguousArrayStorage`. The bridge's `count` *and*
//! `countByEnumeratingWithState:objects:count:` selectors return
//! `NSInteger` (signed, encoding `q`), but `objc2-foundation 0.2.2`
//! declares both with `NSUInteger` (unsigned, `Q`). Runtime encoding
//! validation panics on every call — and crucially it panics on
//! *both* `.count()` and `.iter()`, so there's no way to enumerate the
//! array safely with this bindings version. The CG path sidesteps the
//! whole Swift-array-bridge surface.

#![cfg(target_os = "macos")]

use core_graphics::display::CGDisplay;

/// Look up the backing scale factor for a display. Returns 1.0 if the
/// display is gone or has no active mode (just-disconnected race), or if
/// the mode reports zero logical width (defensive).
pub fn backing_scale_factor_for(display_id: u32) -> f64 {
    let display = CGDisplay::new(display_id);
    let Some(mode) = display.display_mode() else {
        return 1.0;
    };
    let logical = mode.width();
    let physical = mode.pixel_width();
    if logical == 0 {
        return 1.0;
    }
    physical as f64 / logical as f64
}
