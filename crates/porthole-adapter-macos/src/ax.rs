//! RAII wrapper for raw Accessibility (AX) element references.
//!
//! Raw `AXUIElementRef` values are `CFType`-shaped pointers that must be
//! `CFRelease`d by the creator/copier. `AxElement` owns one such pointer
//! and releases it on drop. All FFI calls that produce or consume
//! `AXUIElementRef` should go through this module.

#![cfg(target_os = "macos")]

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

pub type AxError = i32;
pub const AX_ERROR_SUCCESS: AxError = 0;

/// Opaque AX element pointer. Implementation detail — callers should
/// operate through `AxElement` methods.
pub type AxElementRef = *const std::ffi::c_void;

unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AxElementRef,
        attribute: CFStringRef,
        value: *mut *const std::ffi::c_void,
    ) -> AxError;
    fn AXUIElementPerformAction(element: AxElementRef, action: CFStringRef) -> AxError;
    fn _AXUIElementGetWindow(element: AxElementRef, out: *mut u32) -> AxError;
    fn CFRelease(ptr: *const std::ffi::c_void);
}

/// Owned AX element pointer. Drops via `CFRelease`.
///
/// Construct via `AxElement::for_application(pid)` or by wrapping a
/// retained pointer from an AX copy/create call. Never wrap an
/// unretained (get-rule) pointer — it will double-free when the
/// wrapper drops.
pub struct AxElement {
    ptr: AxElementRef,
}

impl AxElement {
    /// Create a top-level application AX element for the given PID.
    /// Returns `None` if the underlying FFI returns null.
    pub fn for_application(pid: i32) -> Option<Self> {
        let ptr = unsafe { AXUIElementCreateApplication(pid) };
        if ptr.is_null() {
            None
        } else {
            Some(Self { ptr })
        }
    }

    /// Wrap a raw retained AXElement pointer. The caller guarantees the
    /// pointer follows the create/copy retain rule (i.e., needs to be
    /// CFRelease'd exactly once by the owner).
    ///
    /// # Safety
    /// Caller must hand over ownership: after this call, do not call
    /// CFRelease on the pointer yourself.
    pub unsafe fn from_retained(ptr: AxElementRef) -> Option<Self> {
        if ptr.is_null() { None } else { Some(Self { ptr }) }
    }

    /// Borrow the raw pointer for FFI calls that need it (e.g.
    /// AXUIElementPerformAction). Must not be used to CFRelease.
    pub fn as_ptr(&self) -> AxElementRef {
        self.ptr
    }

    /// Perform an AX action by name (e.g. "AXPress", "AXRaise").
    pub fn perform_action(&self, action: &str) -> AxError {
        let action_str = CFString::new(action);
        unsafe { AXUIElementPerformAction(self.ptr, action_str.as_concrete_TypeRef() as CFStringRef) }
    }

    /// Copy an attribute value by name. Returns the raw retained pointer
    /// on success — callers wrap it in an appropriate owned type. Returns
    /// None on any error or null value.
    pub fn copy_attribute_raw(&self, attribute: &str) -> Option<*const std::ffi::c_void> {
        let attr_str = CFString::new(attribute);
        let mut out: *const std::ffi::c_void = std::ptr::null();
        let err = unsafe {
            AXUIElementCopyAttributeValue(
                self.ptr,
                attr_str.as_concrete_TypeRef() as CFStringRef,
                &mut out,
            )
        };
        if err == AX_ERROR_SUCCESS && !out.is_null() { Some(out) } else { None }
    }

    /// Look up the CGWindowID for this AX element via the private
    /// `_AXUIElementGetWindow` API. Stable across macOS versions in
    /// widespread use. Returns `None` on any failure.
    pub fn cg_window_id(&self) -> Option<u32> {
        let mut id: u32 = 0;
        let err = unsafe { _AXUIElementGetWindow(self.ptr, &mut id) };
        if err == AX_ERROR_SUCCESS { Some(id) } else { None }
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { CFRelease(self.ptr) };
        }
    }
}

// AXUIElementRef is not Sync by nature (the AX API is main-thread-ish)
// but we wrap the pointer with reasonable care; don't implement Send/Sync.
// Leave the default non-Send/non-Sync behaviour from the raw pointer.

/// Release a raw retained CF pointer. For use with copy-rule pointers
/// (e.g., attribute values copied via `AxElement::copy_attribute_raw`)
/// when they aren't wrapped in an owned type.
pub(crate) unsafe fn cf_release(ptr: *const std::ffi::c_void) {
    if !ptr.is_null() {
        unsafe { CFRelease(ptr) }
    }
}

/// Call `_AXUIElementGetWindow` against a borrowed AX pointer without
/// taking ownership. Used when iterating AX arrays.
pub(crate) unsafe fn ax_get_window_id_borrowed(ptr: AxElementRef) -> Option<u32> {
    let mut id: u32 = 0;
    let err = unsafe { _AXUIElementGetWindow(ptr, &mut id) };
    if err == AX_ERROR_SUCCESS { Some(id) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_from_retained_returns_none() {
        // SAFETY: passing a null pointer; from_retained handles null.
        let e = unsafe { AxElement::from_retained(std::ptr::null()) };
        assert!(e.is_none());
    }

    #[test]
    fn for_application_with_nonexistent_pid_returns_none_or_some() {
        // AXUIElementCreateApplication may return a non-null ref even for
        // a nonexistent PID (it doesn't validate immediately). The value
        // is test-environment dependent, so we only assert it doesn't
        // panic and respects RAII: the returned option drops cleanly.
        let _ = AxElement::for_application(999_999_999);
    }
}
