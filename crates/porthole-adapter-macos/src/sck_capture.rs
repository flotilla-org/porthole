use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{VideoCaptureFrame, VideoCapturePixelFormat, VideoCaptureSession},
    surface::SurfaceInfo,
};
use tokio::sync::mpsc;

use crate::{MacOsAdapter, permissions::ensure_screen_recording_granted};

const K_CV_PIXEL_FORMAT_TYPE_32_BGRA: u32 = u32::from_be_bytes(*b"BGRA");

#[repr(C)]
struct SckFrame {
    data: *const u8,
    len: usize,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: u32,
    timestamp_ns: u64,
}

type FrameCallback = extern "C" fn(*mut c_void, *const SckFrame);
type ErrorCallback = extern "C" fn(*mut c_void, *const c_char);

unsafe extern "C" {
    fn porthole_sck_start_window(
        cg_window_id: u32,
        frame_callback: FrameCallback,
        error_callback: ErrorCallback,
        ctx: *mut c_void,
        out_handle: *mut *mut c_void,
    ) -> *mut c_char;
    fn porthole_sck_stop(handle: *mut c_void);
    fn porthole_sck_free_error(message: *mut c_char);
}

struct CallbackState {
    tx: mpsc::Sender<Result<VideoCaptureFrame, String>>,
    sequence: AtomicU64,
}

pub async fn start_video_capture(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    let cg_window_id = surface.cg_window_id.ok_or_else(|| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "surface has no cg_window_id; cannot start ScreenCaptureKit window stream",
        )
    })?;

    let (tx, rx) = mpsc::channel(8);
    let state = Box::new(CallbackState {
        tx,
        sequence: AtomicU64::new(1),
    });
    let state_ptr = Box::into_raw(state);
    let mut raw_handle = ptr::null_mut();
    let error = unsafe {
        porthole_sck_start_window(
            cg_window_id,
            frame_callback,
            error_callback,
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

    Ok(Box::new(MacVideoCaptureSession {
        raw_handle,
        state: state_ptr,
        rx,
    }))
}

struct MacVideoCaptureSession {
    raw_handle: *mut c_void,
    state: *mut CallbackState,
    rx: mpsc::Receiver<Result<VideoCaptureFrame, String>>,
}

unsafe impl Send for MacVideoCaptureSession {}

#[async_trait]
impl VideoCaptureSession for MacVideoCaptureSession {
    async fn next_frame(&mut self) -> Result<Option<VideoCaptureFrame>, PortholeError> {
        match self.rx.recv().await {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(message)) => Err(PortholeError::new(ErrorCode::CapabilityMissing, message)),
            None => Ok(None),
        }
    }
}

impl Drop for MacVideoCaptureSession {
    fn drop(&mut self) {
        unsafe {
            porthole_sck_stop(self.raw_handle);
            drop(Box::from_raw(self.state));
        }
    }
}

extern "C" fn frame_callback(ctx: *mut c_void, frame: *const SckFrame) {
    if ctx.is_null() || frame.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<CallbackState>()) };
    let frame = unsafe { &*frame };
    if frame.data.is_null() || frame.len == 0 {
        return;
    }
    if frame.pixel_format != K_CV_PIXEL_FORMAT_TYPE_32_BGRA {
        let _ = state
            .tx
            .try_send(Err(format!("unsupported ScreenCaptureKit pixel format {}", frame.pixel_format)));
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(frame.data, frame.len) }.to_vec();
    let sequence = state.sequence.fetch_add(1, Ordering::Relaxed);
    let _ = state.tx.try_send(Ok(VideoCaptureFrame {
        sequence,
        timestamp_ns: frame.timestamp_ns,
        width: frame.width,
        height: frame.height,
        stride: frame.stride,
        pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
        bytes,
    }));
}

extern "C" fn error_callback(ctx: *mut c_void, message: *const c_char) {
    if ctx.is_null() || message.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<CallbackState>()) };
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned();
    let _ = state.tx.try_send(Err(message));
}
