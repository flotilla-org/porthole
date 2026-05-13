use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{
        VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCaptureFrameMetadata, VideoCaptureFramePublisher,
        VideoCaptureFrameView, VideoCapturePixelFormat, VideoCaptureSession, VideoCaptureSyncKind, VideoCaptureTimestampClock,
    },
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
    fn porthole_sck_screenshot_window(
        cg_window_id: u32,
        out_pixels: *mut *mut u8,
        out_len: *mut usize,
        out_width: *mut u32,
        out_height: *mut u32,
        out_bytes_per_row: *mut u32,
    ) -> *mut c_char;
    fn porthole_sck_free_screenshot(pixels: *mut u8);
}

/// One-shot RGBA8 screenshot via SCScreenshotManager. Pixels are laid out
/// `[R, G, B, A]` per element with a tight row stride (`bytes_per_row ==
/// width * 4`), suitable for direct hand-off to the `image` crate's PNG
/// encoder. Caller owns the returned `pixels` Vec; the C buffer is freed
/// before this returns.
pub struct ScreenshotPixels {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bytes_per_row: u32,
}

/// Capture a one-shot screenshot of `cg_window_id` via SCScreenshotManager.
/// macOS 14+. Used by `capture::screenshot` in place of the legacy
/// CGWindowListCreateImage path, which returns null on macOS Tahoe
/// (26 / Darwin 25) even with Screen Recording granted.
///
/// Runs the shim on a blocking task — SCShareableContent's completion
/// handler blocks on a dispatch_semaphore, so calling this directly from a
/// tokio worker would starve the runtime under load.
pub async fn screenshot_window(adapter: &MacOsAdapter, cg_window_id: u32) -> Result<ScreenshotPixels, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    tokio::task::spawn_blocking(move || screenshot_window_blocking(cg_window_id))
        .await
        .map_err(|e| PortholeError::new(ErrorCode::InternalError, format!("screenshot task failed: {e}")))?
}

fn screenshot_window_blocking(cg_window_id: u32) -> Result<ScreenshotPixels, PortholeError> {
    let mut pixels: *mut u8 = ptr::null_mut();
    let mut len: usize = 0;
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut bytes_per_row: u32 = 0;

    let err = unsafe { porthole_sck_screenshot_window(cg_window_id, &mut pixels, &mut len, &mut width, &mut height, &mut bytes_per_row) };

    if !err.is_null() {
        // SAFETY: err is a heap-allocated UTF-8 C string from the shim; we own it.
        let msg = unsafe { CStr::from_ptr(err) }.to_string_lossy().into_owned();
        unsafe { porthole_sck_free_error(err) };
        return Err(PortholeError::new(ErrorCode::CapabilityMissing, msg));
    }

    if pixels.is_null() || len == 0 {
        return Err(PortholeError::new(ErrorCode::InternalError, "screenshot shim returned no pixels"));
    }

    // SAFETY: the shim guarantees `pixels` points to `len` valid bytes when
    // it returns NULL error and non-null pixels. We copy into a Rust-owned
    // Vec before freeing the C buffer so ownership transfer is clean.
    let owned = unsafe { std::slice::from_raw_parts(pixels, len).to_vec() };
    unsafe { porthole_sck_free_screenshot(pixels) };

    Ok(ScreenshotPixels {
        pixels: owned,
        width,
        height,
        bytes_per_row,
    })
}

struct CallbackState {
    tx: mpsc::Sender<Result<VideoCaptureFrame, String>>,
    sequence: AtomicU64,
    dropped_before_publish: AtomicU64,
    producer_drop_count: AtomicU64,
}

struct PublisherCallbackState {
    publisher: Arc<dyn VideoCaptureFramePublisher>,
    error_tx: mpsc::Sender<Result<(), String>>,
    sequence: AtomicU64,
    dropped_before_publish: AtomicU64,
    producer_drop_count: AtomicU64,
}

// SAFETY: CallbackState is shared with SCK callback threads through an opaque C
// ctx pointer. Its fields are thread-safe, and the object is immutable after
// construction except for atomic telemetry counters.
unsafe impl Send for CallbackState {}
// SAFETY: See the Send impl; callbacks only take shared references to state.
unsafe impl Sync for CallbackState {}

// SAFETY: PublisherCallbackState is shared with SCK callback threads through an
// opaque C ctx pointer. Its publisher is Send + Sync, the sender is thread-safe,
// and counters are atomic.
unsafe impl Send for PublisherCallbackState {}
// SAFETY: See the Send impl; callbacks only take shared references to state.
unsafe impl Sync for PublisherCallbackState {}

pub async fn start_video_capture(adapter: &MacOsAdapter, surface: &SurfaceInfo) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    let cg_window_id = surface.cg_window_id.ok_or_else(|| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "surface has no cg_window_id; cannot start ScreenCaptureKit window stream",
        )
    })?;

    let session = tokio::task::spawn_blocking(move || start_video_capture_blocking(cg_window_id))
        .await
        .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("ScreenCaptureKit start task failed: {error}")))??;
    Ok(Box::new(session))
}

pub async fn start_video_capture_publisher(
    adapter: &MacOsAdapter,
    surface: &SurfaceInfo,
    publisher: Arc<dyn VideoCaptureFramePublisher>,
) -> Result<Box<dyn VideoCaptureSession>, PortholeError> {
    ensure_screen_recording_granted(adapter)?;
    let cg_window_id = surface.cg_window_id.ok_or_else(|| {
        PortholeError::new(
            ErrorCode::CapabilityMissing,
            "surface has no cg_window_id; cannot start ScreenCaptureKit window stream",
        )
    })?;

    let session = tokio::task::spawn_blocking(move || start_video_capture_publisher_blocking(cg_window_id, publisher))
        .await
        .map_err(|error| PortholeError::new(ErrorCode::InternalError, format!("ScreenCaptureKit start task failed: {error}")))??;
    Ok(Box::new(session))
}

fn start_video_capture_blocking(cg_window_id: u32) -> Result<MacVideoCaptureSession, PortholeError> {
    let initial_frame = match capture_initial_window_frame(cg_window_id) {
        Ok(frame) => Some(frame),
        Err(error) => {
            tracing::debug!(error = %error, "failed to seed ScreenCaptureKit stream with initial window snapshot");
            None
        }
    };
    let (tx, rx) = mpsc::channel(8);
    let state = Box::new(CallbackState {
        tx: tx.clone(),
        sequence: AtomicU64::new(if initial_frame.is_some() { 2 } else { 1 }),
        dropped_before_publish: AtomicU64::new(0),
        producer_drop_count: AtomicU64::new(0),
    });
    let state_ptr = Box::into_raw(state);
    let mut raw_handle = ptr::null_mut();
    if let Some(frame) = initial_frame {
        // Queue the seed frame before SCK starts so consumers observe sequence 1 before live callbacks.
        let _ = tx.try_send(Ok(frame));
    }
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
    Ok(MacVideoCaptureSession {
        raw_handle,
        state: state_ptr,
        rx,
    })
}

fn start_video_capture_publisher_blocking(
    cg_window_id: u32,
    publisher: Arc<dyn VideoCaptureFramePublisher>,
) -> Result<MacPublishedVideoCaptureSession, PortholeError> {
    let initial_frame = match capture_initial_window_frame(cg_window_id) {
        Ok(frame) => Some(frame),
        Err(error) => {
            tracing::debug!(error = %error, "failed to seed ScreenCaptureKit publisher stream with initial window snapshot");
            None
        }
    };
    let (error_tx, error_rx) = mpsc::channel(8);
    let state = Box::new(PublisherCallbackState {
        publisher,
        error_tx,
        sequence: AtomicU64::new(if initial_frame.is_some() { 2 } else { 1 }),
        dropped_before_publish: AtomicU64::new(0),
        producer_drop_count: AtomicU64::new(0),
    });
    if let Some(frame) = initial_frame.as_ref() {
        state.publisher.publish_frame(frame.as_view())?;
    }
    let state_ptr = Box::into_raw(state);
    let mut raw_handle = ptr::null_mut();
    let error = unsafe {
        porthole_sck_start_window(
            cg_window_id,
            publisher_frame_callback,
            publisher_error_callback,
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
    Ok(MacPublishedVideoCaptureSession {
        raw_handle,
        state: state_ptr,
        rx: error_rx,
    })
}

fn capture_initial_window_frame(cg_window_id: u32) -> Result<VideoCaptureFrame, PortholeError> {
    use std::time::{SystemTime, UNIX_EPOCH};

    use core_graphics::{
        geometry::{CGPoint, CGRect, CGSize},
        window::{create_image, kCGWindowImageBoundsIgnoreFraming, kCGWindowImageDefault, kCGWindowListOptionIncludingWindow},
    };

    let zero_rect = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0));
    let image = create_image(
        zero_rect,
        kCGWindowListOptionIncludingWindow,
        cg_window_id,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageDefault,
    )
    .ok_or_else(|| PortholeError::new(ErrorCode::CapabilityMissing, "initial capture snapshot returned null"))?;

    let width = image.width() as u32;
    let height = image.height() as u32;
    let bytes_per_row = image.bytes_per_row();
    let data = image.data();
    let bgra: &[u8] = data.bytes();
    let stride = width
        .checked_mul(4)
        .ok_or_else(|| PortholeError::new(ErrorCode::InternalError, "initial capture snapshot stride overflow"))?;
    let mut bytes = Vec::with_capacity((stride as usize) * (height as usize));
    for row in 0..height as usize {
        let row_start = row * bytes_per_row;
        let row_end = row_start + stride as usize;
        if row_end > bgra.len() {
            return Err(PortholeError::new(
                ErrorCode::InternalError,
                "initial capture snapshot bytes shorter than expected",
            ));
        }
        bytes.extend_from_slice(&bgra[row_start..row_end]);
    }
    drop(data);
    drop(image);
    let timestamp_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);

    let metadata = frame_metadata(CaptureFrameSource::CoreGraphicsSeed, 1, 0, 0);
    Ok(VideoCaptureFrame {
        sequence: 1,
        timestamp_ns,
        timestamp_clock: metadata.timestamp_clock,
        width,
        height,
        stride,
        pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
        color_space: metadata.color_space,
        sync_kind: metadata.sync_kind,
        damage_kind: metadata.damage_kind,
        damage_base_sequence: metadata.damage_base_sequence,
        dropped_before_publish: metadata.dropped_before_publish,
        producer_drop_count: metadata.producer_drop_count,
        bytes,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureFrameSource {
    CoreGraphicsSeed,
    ScreenCaptureKitLive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureFrameMetadata {
    timestamp_clock: VideoCaptureTimestampClock,
    color_space: VideoCaptureColorSpace,
    sync_kind: VideoCaptureSyncKind,
    damage_kind: VideoCaptureDamageKind,
    damage_base_sequence: u64,
    dropped_before_publish: u64,
    producer_drop_count: u64,
}

fn frame_metadata(
    source: CaptureFrameSource,
    sequence: u64,
    dropped_before_publish: u64,
    producer_drop_count: u64,
) -> CaptureFrameMetadata {
    let (timestamp_clock, sync_kind) = match source {
        CaptureFrameSource::CoreGraphicsSeed => (VideoCaptureTimestampClock::UnixTime, VideoCaptureSyncKind::CpuCopyComplete),
        CaptureFrameSource::ScreenCaptureKitLive => (VideoCaptureTimestampClock::MediaTime, VideoCaptureSyncKind::SckSampleReady),
    };
    CaptureFrameMetadata {
        timestamp_clock,
        color_space: VideoCaptureColorSpace::Unknown,
        sync_kind,
        damage_kind: VideoCaptureDamageKind::FullFrame,
        damage_base_sequence: sequence,
        dropped_before_publish,
        producer_drop_count,
    }
}

struct MacVideoCaptureSession {
    raw_handle: *mut c_void,
    state: *mut CallbackState,
    rx: mpsc::Receiver<Result<VideoCaptureFrame, String>>,
}

struct MacPublishedVideoCaptureSession {
    raw_handle: *mut c_void,
    state: *mut PublisherCallbackState,
    rx: mpsc::Receiver<Result<(), String>>,
}

// SAFETY: MacVideoCaptureSession owns the SCK handle and CallbackState pointer.
// CallbackState is immutable after construction except for its AtomicU64; the
// Sender is thread-safe, and Drop clears callbacks before freeing state.
unsafe impl Send for MacVideoCaptureSession {}

// SAFETY: MacPublishedVideoCaptureSession owns the SCK handle and
// PublisherCallbackState pointer. Drop clears callbacks before freeing state.
unsafe impl Send for MacPublishedVideoCaptureSession {}

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

#[async_trait]
impl VideoCaptureSession for MacPublishedVideoCaptureSession {
    async fn next_frame(&mut self) -> Result<Option<VideoCaptureFrame>, PortholeError> {
        match self.rx.recv().await {
            Some(Err(message)) => Err(PortholeError::new(ErrorCode::CapabilityMissing, message)),
            Some(Ok(())) => Ok(None),
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

impl Drop for MacPublishedVideoCaptureSession {
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
    let dropped_before_publish = state.dropped_before_publish.swap(0, Ordering::Relaxed);
    let producer_drop_count = state.producer_drop_count.load(Ordering::Relaxed);
    let metadata = frame_metadata(
        CaptureFrameSource::ScreenCaptureKitLive,
        sequence,
        dropped_before_publish,
        producer_drop_count,
    );
    // Capacity is intentionally small; slow consumers drop frames and catch the next one.
    if state
        .tx
        .try_send(Ok(VideoCaptureFrame {
            sequence,
            timestamp_ns: frame.timestamp_ns,
            timestamp_clock: metadata.timestamp_clock,
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: metadata.color_space,
            sync_kind: metadata.sync_kind,
            damage_kind: metadata.damage_kind,
            damage_base_sequence: metadata.damage_base_sequence,
            dropped_before_publish: metadata.dropped_before_publish,
            producer_drop_count: metadata.producer_drop_count,
            bytes,
        }))
        .is_err()
    {
        restore_failed_publish_drop_run(&state.dropped_before_publish, dropped_before_publish);
        state.producer_drop_count.fetch_add(1, Ordering::Relaxed);
    }
}

extern "C" fn publisher_frame_callback(ctx: *mut c_void, frame: *const SckFrame) {
    if ctx.is_null() || frame.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<PublisherCallbackState>()) };
    let frame = unsafe { &*frame };
    if frame.data.is_null() || frame.len == 0 {
        return;
    }
    if frame.pixel_format != K_CV_PIXEL_FORMAT_TYPE_32_BGRA {
        let _ = state
            .error_tx
            .try_send(Err(format!("unsupported ScreenCaptureKit pixel format {}", frame.pixel_format)));
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(frame.data, frame.len) };
    let sequence = state.sequence.fetch_add(1, Ordering::Relaxed);
    let dropped_before_publish = state.dropped_before_publish.swap(0, Ordering::Relaxed);
    let producer_drop_count = state.producer_drop_count.load(Ordering::Relaxed);
    let metadata = frame_metadata(
        CaptureFrameSource::ScreenCaptureKitLive,
        sequence,
        dropped_before_publish,
        producer_drop_count,
    );
    let view = VideoCaptureFrameView {
        metadata: VideoCaptureFrameMetadata {
            sequence,
            timestamp_ns: frame.timestamp_ns,
            timestamp_clock: metadata.timestamp_clock,
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
            color_space: metadata.color_space,
            sync_kind: metadata.sync_kind,
            damage_kind: metadata.damage_kind,
            damage_base_sequence: metadata.damage_base_sequence,
            dropped_before_publish: metadata.dropped_before_publish,
            producer_drop_count: metadata.producer_drop_count,
        },
        bytes,
    };
    if let Err(error) = state.publisher.publish_frame(view) {
        let _ = state.error_tx.try_send(Err(error.to_string()));
        restore_failed_publish_drop_run(&state.dropped_before_publish, dropped_before_publish);
        state.producer_drop_count.fetch_add(1, Ordering::Relaxed);
    }
}

fn restore_failed_publish_drop_run(counter: &AtomicU64, claimed_drop_run: u64) {
    // The preceding swap gives exactly one callback ownership of the observed
    // drop run. If publishing fails, put that run back and include this failed
    // frame so the next successful publish reports all missed frames.
    counter.fetch_add(claimed_drop_run.saturating_add(1), Ordering::Relaxed);
}

extern "C" fn error_callback(ctx: *mut c_void, message: *const c_char) {
    if ctx.is_null() || message.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<CallbackState>()) };
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned();
    let _ = state.tx.try_send(Err(message));
}

extern "C" fn publisher_error_callback(ctx: *mut c_void, message: *const c_char) {
    if ctx.is_null() || message.is_null() {
        return;
    }
    let state = unsafe { &*(ctx.cast::<PublisherCallbackState>()) };
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy().into_owned();
    let _ = state.error_tx.try_send(Err(message));
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use porthole_core::{
        PortholeError,
        adapter::{
            VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrameMetadata, VideoCaptureFramePublisher, VideoCaptureFrameView,
            VideoCaptureSyncKind, VideoCaptureTimestampClock,
        },
    };
    use tokio::sync::mpsc;

    use super::{
        CaptureFrameMetadata, CaptureFrameSource, K_CV_PIXEL_FORMAT_TYPE_32_BGRA, PublisherCallbackState, SckFrame, frame_metadata,
        publisher_frame_callback, restore_failed_publish_drop_run,
    };

    #[derive(Debug, Default)]
    struct RecordingPublisher {
        frames: Mutex<Vec<(VideoCaptureFrameMetadata, Vec<u8>)>>,
    }

    impl VideoCaptureFramePublisher for RecordingPublisher {
        fn publish_frame(&self, frame: VideoCaptureFrameView<'_>) -> Result<(), PortholeError> {
            self.frames.lock().unwrap().push((frame.metadata, frame.bytes.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn seed_frame_metadata_uses_unix_time_and_cpu_completion() {
        let metadata = frame_metadata(CaptureFrameSource::CoreGraphicsSeed, 1, 0, 0);

        assert_eq!(
            metadata,
            CaptureFrameMetadata {
                timestamp_clock: VideoCaptureTimestampClock::UnixTime,
                color_space: VideoCaptureColorSpace::Unknown,
                sync_kind: VideoCaptureSyncKind::CpuCopyComplete,
                damage_kind: VideoCaptureDamageKind::FullFrame,
                damage_base_sequence: 1,
                dropped_before_publish: 0,
                producer_drop_count: 0,
            }
        );
    }

    #[test]
    fn live_frame_metadata_uses_media_time_and_sck_ready() {
        let metadata = frame_metadata(CaptureFrameSource::ScreenCaptureKitLive, 4, 2, 3);

        assert_eq!(
            metadata,
            CaptureFrameMetadata {
                timestamp_clock: VideoCaptureTimestampClock::MediaTime,
                color_space: VideoCaptureColorSpace::Unknown,
                sync_kind: VideoCaptureSyncKind::SckSampleReady,
                damage_kind: VideoCaptureDamageKind::FullFrame,
                damage_base_sequence: 4,
                dropped_before_publish: 2,
                producer_drop_count: 3,
            }
        );
    }

    #[test]
    fn publisher_frame_callback_forwards_live_frame_view() {
        let publisher = Arc::new(RecordingPublisher::default());
        let (error_tx, _error_rx) = mpsc::channel(1);
        let state = PublisherCallbackState {
            publisher: publisher.clone(),
            error_tx,
            sequence: std::sync::atomic::AtomicU64::new(4),
            dropped_before_publish: std::sync::atomic::AtomicU64::new(2),
            producer_drop_count: std::sync::atomic::AtomicU64::new(3),
        };
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8];
        let frame = SckFrame {
            data: pixels.as_ptr(),
            len: pixels.len(),
            width: 2,
            height: 1,
            stride: 8,
            pixel_format: K_CV_PIXEL_FORMAT_TYPE_32_BGRA,
            timestamp_ns: 999,
        };

        publisher_frame_callback((&raw const state).cast::<std::ffi::c_void>().cast_mut(), &frame);

        let frames = publisher.frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.sequence, 4);
        assert_eq!(frames[0].0.timestamp_ns, 999);
        assert_eq!(frames[0].0.width, 2);
        assert_eq!(frames[0].0.sync_kind, VideoCaptureSyncKind::SckSampleReady);
        assert_eq!(frames[0].1, pixels);
    }

    #[test]
    fn failed_publish_drop_restore_does_not_double_count_racing_claims() {
        let counter = std::sync::atomic::AtomicU64::new(2);

        let first_claim = counter.swap(0, std::sync::atomic::Ordering::Relaxed);
        let second_claim = counter.swap(0, std::sync::atomic::Ordering::Relaxed);
        restore_failed_publish_drop_run(&counter, first_claim);
        restore_failed_publish_drop_run(&counter, second_claim);

        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 4);
    }
}
