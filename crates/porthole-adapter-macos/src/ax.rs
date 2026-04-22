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
    /// Private AXValue helper — extracts a CGPoint or CGSize from an AXValueRef.
    pub(crate) fn AXValueGetValue(
        value: *const std::ffi::c_void,
        the_type: i32,
        value_ptr: *mut std::ffi::c_void,
    ) -> u8;
    fn AXUIElementSetAttributeValue(
        element: AxElementRef,
        attribute: CFStringRef,
        value: *const std::ffi::c_void,
    ) -> AxError;
    fn AXValueCreate(the_type: i32, value_ptr: *const std::ffi::c_void) -> *const std::ffi::c_void;
}

/// AXValue type tag for CGPoint (matches macOS ApplicationServices/AXValue.h).
pub const AX_VALUE_CG_POINT: i32 = 1;
/// AXValue type tag for CGSize.
pub const AX_VALUE_CG_SIZE: i32 = 2;
/// AXValue type tag for CGRect.
pub const AX_VALUE_CG_RECT: i32 = 3;

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

    /// Write an AXValue-wrapped `CGPoint` to the `AXPosition` attribute.
    /// Returns the AX error code (0 = success).
    pub fn set_position(&self, x: f64, y: f64) -> AxError {
        use core_graphics::geometry::CGPoint;
        let pt = CGPoint::new(x, y);
        unsafe {
            let value =
                AXValueCreate(AX_VALUE_CG_POINT, &pt as *const _ as *const std::ffi::c_void);
            if value.is_null() {
                return -1;
            }
            let attr = CFString::new("AXPosition");
            let err = AXUIElementSetAttributeValue(
                self.ptr,
                attr.as_concrete_TypeRef() as CFStringRef,
                value,
            );
            CFRelease(value);
            err
        }
    }

    /// Write an AXValue-wrapped `CGSize` to the `AXSize` attribute.
    /// Returns the AX error code (0 = success).
    pub fn set_size(&self, w: f64, h: f64) -> AxError {
        use core_graphics::geometry::CGSize;
        let sz = CGSize::new(w, h);
        unsafe {
            let value =
                AXValueCreate(AX_VALUE_CG_SIZE, &sz as *const _ as *const std::ffi::c_void);
            if value.is_null() {
                return -1;
            }
            let attr = CFString::new("AXSize");
            let err = AXUIElementSetAttributeValue(
                self.ptr,
                attr.as_concrete_TypeRef() as CFStringRef,
                value,
            );
            CFRelease(value);
            err
        }
    }

    /// Read the `AXPosition` attribute as a `(x, y)` pair.
    /// Returns `None` if the attribute is absent or conversion fails.
    pub fn get_position(&self) -> Option<(f64, f64)> {
        use core_graphics::geometry::CGPoint;
        let raw = self.copy_attribute_raw("AXPosition")?;
        let mut pt = CGPoint::new(0.0, 0.0);
        let ok = unsafe {
            AXValueGetValue(
                raw,
                AX_VALUE_CG_POINT,
                &mut pt as *mut _ as *mut std::ffi::c_void,
            )
        };
        unsafe { cf_release(raw) };
        if ok != 0 { Some((pt.x, pt.y)) } else { None }
    }

    /// Read the `AXSize` attribute as a `(width, height)` pair.
    /// Returns `None` if the attribute is absent or conversion fails.
    pub fn get_size(&self) -> Option<(f64, f64)> {
        use core_graphics::geometry::CGSize;
        let raw = self.copy_attribute_raw("AXSize")?;
        let mut sz = CGSize::new(0.0, 0.0);
        let ok = unsafe {
            AXValueGetValue(
                raw,
                AX_VALUE_CG_SIZE,
                &mut sz as *mut _ as *mut std::ffi::c_void,
            )
        };
        unsafe { cf_release(raw) };
        if ok != 0 { Some((sz.width, sz.height)) } else { None }
    }

    /// Run `op` against a borrowed AX pointer without taking ownership.
    /// The element is NOT released at the end of the closure.
    /// Prefer this over raw pointer manipulation to avoid double-release.
    ///
    /// # Safety
    /// `ptr` must be a valid, live `AXUIElementRef` that outlives the closure.
    pub unsafe fn with_borrowed<F, R>(ptr: AxElementRef, op: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        let tmp = Self { ptr };
        let r = op(&tmp);
        std::mem::forget(tmp);
        r
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

/// Copy an attribute from a *borrowed* (unowned) `AxElementRef`. The returned
/// pointer (if `Some`) is retained and must be released by the caller via
/// `cf_release`.
///
/// # Safety
/// `ptr` must be a valid, live AXUIElementRef. It is not consumed or released.
pub(crate) unsafe fn copy_attribute_borrowed(
    ptr: AxElementRef,
    attribute: &str,
) -> Option<*const std::ffi::c_void> {
    let attr_str = CFString::new(attribute);
    let mut out: *const std::ffi::c_void = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(
            ptr,
            attr_str.as_concrete_TypeRef() as CFStringRef,
            &mut out,
        )
    };
    if err == AX_ERROR_SUCCESS && !out.is_null() { Some(out) } else { None }
}

/// Perform an AX action on a *borrowed* (unowned) `AxElementRef`.
///
/// # Safety
/// `ptr` must be a valid, live AXUIElementRef. It is not consumed or released.
pub(crate) unsafe fn perform_action_borrowed(ptr: AxElementRef, action: &str) {
    let action_str = CFString::new(action);
    unsafe {
        let _ = AXUIElementPerformAction(ptr, action_str.as_concrete_TypeRef() as CFStringRef);
    }
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
