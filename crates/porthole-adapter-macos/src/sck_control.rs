//! Owned SCK control shared by CPU and native streams. Stream destruction
//! serializes with submitting an update, but does not wait for its completion.
use std::{
    ffi::{CStr, c_char, c_void},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use porthole_core::{
    ErrorCode, PortholeError,
    adapter::{VideoCaptureOutputControl, VideoCaptureOutputSize},
};
use tokio::sync::{OwnedMutexGuard, oneshot};

unsafe extern "C" {
    fn porthole_sck_set_output_size(
        handle: *mut c_void,
        width: u32,
        height: u32,
        callback: extern "C" fn(*mut c_void, *const c_char),
        ctx: *mut c_void,
    );
    fn porthole_sck_stop(handle: *mut c_void);
}

#[derive(Debug)]
struct Handle(*mut c_void);
// SAFETY: every use of the retained ObjC handle is serialized by `handle`.
// The shim permits calls from arbitrary threads, outside its sample queue.
unsafe impl Send for Handle {}

#[derive(Debug)]
pub(crate) struct SckControl {
    handle: Mutex<Option<Handle>>,
    updating: Arc<tokio::sync::Mutex<()>>,
}

impl SckControl {
    /// Takes ownership of a non-null, retained SCK shim handle. The stream must
    /// call stop before destroying its callback context, even with control clones.
    pub(crate) unsafe fn new(handle: *mut c_void) -> Arc<Self> {
        assert!(!handle.is_null());
        Arc::new(Self {
            handle: Mutex::new(Some(Handle(handle))),
            updating: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    pub(crate) fn stop(&self) {
        let mut handle = self.handle.lock().expect("SCK control poisoned");
        if let Some(handle) = handle.take() {
            // Clears and joins sample callbacks before their Rust state is freed.
            unsafe { porthole_sck_stop(handle.0) };
        }
    }
}

struct UpdateCompletion {
    tx: oneshot::Sender<Result<(), PortholeError>>,
    // Held by the backend callback, even if the requesting future is cancelled.
    _updating: OwnedMutexGuard<()>,
}

extern "C" fn update_complete(ctx: *mut c_void, error: *const c_char) {
    // SAFETY: the shim calls this exactly once, with the owned box and a
    // borrowed error string that remains valid throughout this callback.
    let completion = unsafe { Box::from_raw(ctx.cast::<UpdateCompletion>()) };
    let result = if error.is_null() {
        Ok(())
    } else {
        Err(PortholeError::new(
            ErrorCode::CapabilityMissing,
            unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned(),
        ))
    };
    let _ = completion.tx.send(result);
}

#[async_trait]
impl VideoCaptureOutputControl for SckControl {
    async fn set_output_size(&self, size: VideoCaptureOutputSize) -> Result<(), PortholeError> {
        if size.width == 0 || size.height == 0 {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                "capture output dimensions must be positive pixels",
            ));
        }
        let updating = self
            .updating
            .clone()
            .try_lock_owned()
            .map_err(|_| PortholeError::new(ErrorCode::InvalidArgument, "capture output update is already in progress"))?;
        let (tx, rx) = oneshot::channel();
        {
            let handle = self.handle.lock().expect("SCK control poisoned");
            let handle = handle
                .as_ref()
                .ok_or_else(|| PortholeError::new(ErrorCode::InvalidArgument, "capture stream is closed"))?;
            let completion = Box::into_raw(Box::new(UpdateCompletion { tx, _updating: updating }));
            // The call only submits. Its completion owns all asynchronous state;
            // it neither borrows the stream's frame context nor dereferences handle.
            unsafe { porthole_sck_set_output_size(handle.0, size.width, size.height, update_complete, completion.cast()) };
        }
        rx.await
            .map_err(|_| PortholeError::new(ErrorCode::InternalError, "capture output completion lost"))??;
        if self.handle.lock().expect("SCK control poisoned").is_none() {
            return Err(PortholeError::new(
                ErrorCode::InvalidArgument,
                "capture stream closed during output update",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_request_keeps_update_reserved_until_backend_completion() {
        let updating = Arc::new(tokio::sync::Mutex::new(()));
        let (tx, rx) = oneshot::channel();
        let completion = Box::into_raw(Box::new(UpdateCompletion {
            tx,
            _updating: updating.clone().try_lock_owned().unwrap(),
        }));
        drop(rx);
        assert!(updating.clone().try_lock_owned().is_err());
        update_complete(completion.cast(), std::ptr::null());
        assert!(updating.try_lock_owned().is_ok());
    }

    #[tokio::test]
    async fn closed_control_cannot_submit_or_reopen_stream() {
        let control = SckControl {
            handle: Mutex::new(None),
            updating: Arc::new(tokio::sync::Mutex::new(())),
        };
        assert_eq!(
            control
                .set_output_size(VideoCaptureOutputSize { width: 20, height: 10 })
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert!(control.updating.try_lock().is_ok());
    }
}
