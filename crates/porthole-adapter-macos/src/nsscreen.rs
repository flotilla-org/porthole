//! CGDirectDisplayID → NSScreen.backingScaleFactor lookup.
//!
//! `CGDisplayPixelsWide` on modern macOS returns the logical width of the
//! active display mode (i.e., points), not the backing pixel count, so it
//! cannot be used to compute the backing scale factor. `NSScreen.back-
//! ingScaleFactor` is the authoritative source — this module bridges the
//! CGDirectDisplayID we have to the matching `NSScreen` and reads it.

#![cfg(target_os = "macos")]

use objc2_app_kit::NSScreen;
use objc2_foundation::{MainThreadMarker, NSNumber, NSString};

/// Look up the backing scale factor for a display. Returns 1.0 if the
/// screen can't be found (e.g., just disconnected between our display
/// enumeration and this call).
pub fn backing_scale_factor_for(display_id: u32) -> f64 {
    // SAFETY: NSScreen enumeration is safe to call from background threads
    // on macOS in practice. We construct the marker to satisfy the type
    // system; the underlying Obj-C calls are thread-safe for read-only
    // display queries.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    unsafe {
        let screens = NSScreen::screens(mtm);
        let count = screens.count();
        for i in 0..count {
            let screen = screens.objectAtIndex(i);
            let device_description = screen.deviceDescription();
            let screen_number_key = NSString::from_str("NSScreenNumber");
            let value = device_description.objectForKey(&*screen_number_key);
            let Some(value) = value else { continue };
            // Cast AnyObject → NSNumber to read the numeric display ID.
            let number: objc2::rc::Retained<NSNumber> =
                objc2::rc::Retained::cast(value);
            let this_id = number.unsignedIntValue();
            if this_id == display_id {
                return screen.backingScaleFactor();
            }
        }
        1.0
    }
}
