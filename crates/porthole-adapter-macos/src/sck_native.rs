//! Native-handle SCK capture (#84): each frame is delivered as the stream's
//! backing IOSurface — no pixel lock, no CPU copy. The CPU path in
//! [`crate::sck_capture`] stays as the fallback for consumers that want
//! bytes.
//!
//! This seam is macOS-specific by design: a native frame *is* an IOSurface,
//! so unlike the byte-oriented [`VideoCaptureFramePublisher`] there is
//! nothing platform-neutral to abstract here. The publisher (portholed's
//! native track producer) blits the captured surface into its own pool and
//! publishes a ring descriptor; sequencing and drop accounting live there,
//! not in this delivery layer.
//!
//! [`VideoCaptureFramePublisher`]: porthole_core::adapter::VideoCaptureFramePublisher

use std::{
    ffi::{CStr, c_char, c_void},
    ptr::{self, NonNull},
    sync::Arc,
};

use capture_transfer::native::macos::IoSurface;
use porthole_core::{ErrorCode, PortholeError, surface::SurfaceInfo};

use crate::{MacOsAdapter, permissions::ensure_screen_recording_granted};

#[repr(C)]
struct SckNativeFrame {
    io_surface: *const c_void,
    width: u32,
    height: u32,
    pixel_format: u32,
    timestamp_ns: u64,
}

type NativeFrameCallback = extern "C" fn(*mut c_void, *const SckNativeFrame);
type ErrorCallback = extern "C" fn(*mut c_void, *const c_char);

unsafe extern "C" {
    fn porthole_sck_start_window_native(
        cg_window_id: u32,
        frame_callback: NativeFrameCallback,
        error_callback: ErrorCallback,
        ctx: *mut c_void,
        out_handle: *mut *mut c_void,
    ) -> *mut c_char;
    fn porthole_sck_stop(handle: *mut c_void);
    fn porthole_sck_free_error(message: *mut c_char);
}

/// One live frame: the retained backing surface plus the sample's metadata.
/// `timestamp_ns` is media time (the SCK presentation timestamp).
#[derive(Debug)]
pub struct NativeCapturedFrame {
    pub surface: IoSurface,
    pub width: u32,
    pub height: u32,
    /// CoreVideo fourcc (`kCVPixelFormatType_32BGRA` for SCK window streams).
    pub pixel_format: u32,
    pub timestamp_ns: u64,
}

/// Receives native frames on the SCK sample-handler queue. Implementations
/// own publish policy (staging, drops, resize handling); delivery never
/// blocks on policy.
pub trait NativeVideoFramePublisher: Send + Sync {
    fn publish_native_frame(&self, frame: NativeCapturedFrame);
    /// Stream-level errors. The stream may keep delivering afterwards (e.g.
    /// a single bad sample) or may be dead (SCK stop); the publisher decides
    /// how to surface it.
    fn capture_error(&self, message: &str);
}

struct NativeCallbackState {
    publisher: Arc<dyn NativeVideoFramePublisher>,
}

/// A running native capture stream. Dropping it stops the SCK stream and
/// tears down the callback state.
pub struct NativeSckCaptureStream {
    raw_handle: *mut c_void,
    state: *mut NativeCallbackState,
}

// SAFETY: NativeSckCaptureStream owns the SCK handle and callback state.
// Drop clears the shim callbacks (porthole_sck_stop dispatch_syncs on the
// sample queue) before freeing state.
unsafe impl Send for NativeSckCaptureStream {}

impl Drop for NativeSckCaptureStream {
    fn drop(&mut self) {
        unsafe {
            porthole_sck_stop(self.raw_handle);
            drop(Box::from_raw(self.state));
        }
    }
}

impl std::fmt::Debug for NativeSckCaptureStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeSckCaptureStream").finish_non_exhaustive()
    }
}

/// Start a native window capture stream delivering IOSurfaces to
/// `publisher`. Requires Screen Recording.
pub async fn start_native_window_capture(
    adapter: &MacOsAdapter,
    surface: &SurfaceInfo,
    publisher: Arc<dyn NativeVideoFramePublisher>,
) -> Result<NativeSckCaptureStream, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    let cg_window_id = surface.macos_cg_window_id().ok_or_else(|| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "surface has no cg_window_id; cannot start ScreenCaptureKit window stream",
        )
    })?;
    tokio::task::spawn_blocking(move || start_native_window_capture_blocking(cg_window_id, publisher))
        .await
        .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("ScreenCaptureKit start task failed: {error}")))?
}

fn start_native_window_capture_blocking(
    cg_window_id: u32,
    publisher: Arc<dyn NativeVideoFramePublisher>,
) -> Result<NativeSckCaptureStream, PortholeError> {
    let state_ptr = Box::into_raw(Box::new(NativeCallbackState { publisher }));
    let mut raw_handle = ptr::null_mut();
    let error = unsafe {
        porthole_sck_start_window_native(
            cg_window_id,
            native_frame_callback,
            native_error_callback,
            state_ptr.cast::<c_void>(),
            &mut raw_handle,
        )
    };
    if !error.is_null() {
        let message = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
        unsafe {
            porthole_sck_free_error(error);
            drop(Box::from_raw(state_ptr));
        }
        return Err(PortholeError::new(ErrorCode::CapabilityMissing, message));
    }
    if raw_handle.is_null() {
        unsafe {
            drop(Box::from_raw(state_ptr));
        }
        return Err(PortholeError::new(
            ErrorCode::CapabilityMissing,
            "ScreenCaptureKit did not return a stream handle",
        ));
    }
    Ok(NativeSckCaptureStream {
        raw_handle,
        state: state_ptr,
    })
}

extern "C" fn native_frame_callback(ctx: *mut c_void, frame: *const SckNativeFrame) {
    if ctx.is_null() || frame.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<NativeCallbackState>()) };
    let frame = unsafe { &*frame };
    let Some(raw) = NonNull::new(frame.io_surface.cast_mut()) else {
        state.publisher.capture_error("ScreenCaptureKit delivered a NULL IOSurface");
        return;
    };
    // The shim borrows the surface for the duration of the callback;
    // from_borrowed retains it so the frame outlives the sample buffer.
    let surface = unsafe { IoSurface::from_borrowed(raw) };
    state.publisher.publish_native_frame(NativeCapturedFrame {
        surface,
        width: frame.width,
        height: frame.height,
        pixel_format: frame.pixel_format,
        timestamp_ns: frame.timestamp_ns,
    });
}

extern "C" fn native_error_callback(ctx: *mut c_void, message: *const c_char) {
    if ctx.is_null() || message.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<NativeCallbackState>()) };
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    state.publisher.capture_error(&message);
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use capture_transfer::{model::PixelFormat, native::macos::IoSurface};

    use super::{
        NativeCallbackState, NativeCapturedFrame, NativeVideoFramePublisher, SckNativeFrame, native_error_callback, native_frame_callback,
    };

    #[derive(Default)]
    struct RecordingPublisher {
        frames: Mutex<Vec<NativeCapturedFrame>>,
        errors: Mutex<Vec<String>>,
    }

    impl NativeVideoFramePublisher for RecordingPublisher {
        fn publish_native_frame(&self, frame: NativeCapturedFrame) {
            self.frames.lock().unwrap().push(frame);
        }

        fn capture_error(&self, message: &str) {
            self.errors.lock().unwrap().push(message.to_string());
        }
    }

    #[test]
    fn frame_callback_retains_the_surface_beyond_the_callback() {
        let publisher = Arc::new(RecordingPublisher::default());
        let state = NativeCallbackState {
            publisher: Arc::clone(&publisher) as _,
        };

        let pixels: Vec<u8> = (0..32 * 16 * 4).map(|i| i as u8).collect();
        let captured = {
            // The "SCK-owned" surface lives only in this scope, like a
            // sample buffer released after the callback returns.
            let sck_surface = IoSurface::allocate(32, 16, PixelFormat::Bgra8Unorm).unwrap();
            sck_surface.write_pixels(&pixels).unwrap();
            let frame = SckNativeFrame {
                io_surface: sck_surface.as_raw(),
                width: 32,
                height: 16,
                pixel_format: u32::from_be_bytes(*b"BGRA"),
                timestamp_ns: 42,
            };
            native_frame_callback((&raw const state).cast_mut().cast(), &raw const frame);
            drop(sck_surface);
            publisher.frames.lock().unwrap().pop().expect("frame not delivered")
        };

        assert_eq!((captured.width, captured.height), (32, 16));
        assert_eq!(captured.timestamp_ns, 42);
        let mut read_back = vec![0u8; pixels.len()];
        captured.surface.read_pixels(&mut read_back).unwrap();
        assert_eq!(read_back, pixels, "retained surface must outlive the SCK sample");
    }

    #[test]
    fn error_callback_forwards_message() {
        let publisher = Arc::new(RecordingPublisher::default());
        let state = NativeCallbackState {
            publisher: Arc::clone(&publisher) as _,
        };
        let message = std::ffi::CString::new("stream stopped").unwrap();
        native_error_callback((&raw const state).cast_mut().cast(), message.as_ptr());
        assert_eq!(publisher.errors.lock().unwrap().as_slice(), ["stream stopped"]);
    }

    #[test]
    fn null_surface_is_an_error_not_a_frame() {
        let publisher = Arc::new(RecordingPublisher::default());
        let state = NativeCallbackState {
            publisher: Arc::clone(&publisher) as _,
        };
        let frame = SckNativeFrame {
            io_surface: std::ptr::null(),
            width: 1,
            height: 1,
            pixel_format: 0,
            timestamp_ns: 0,
        };
        native_frame_callback((&raw const state).cast_mut().cast(), &raw const frame);
        assert!(publisher.frames.lock().unwrap().is_empty());
        assert_eq!(publisher.errors.lock().unwrap().len(), 1);
    }
}
