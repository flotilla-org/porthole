use std::{
    ffi::{CStr, CString, c_char, c_int, c_uchar, c_void},
    ptr::NonNull,
};

use crate::{
    client::ClientError,
    commands::record::{MovieWriterSettings, validate_movie_writer_settings},
};

pub struct AvMovieWriter {
    raw: NonNull<c_void>,
    finished: bool,
    stride: usize,
    height: usize,
}

unsafe extern "C" {
    fn porthole_record_av_writer_create(
        path: *const c_char,
        width: u32,
        height: u32,
        stride: u32,
        out_writer: *mut *mut c_void,
        out_error: *mut *mut c_char,
    ) -> c_int;
    fn porthole_record_av_writer_append(
        writer: *mut c_void,
        timestamp_ns: u64,
        bytes: *const c_uchar,
        len: usize,
        out_error: *mut *mut c_char,
    ) -> c_int;
    fn porthole_record_av_writer_finish(writer: *mut c_void, out_error: *mut *mut c_char) -> c_int;
    fn porthole_record_av_writer_destroy(writer: *mut c_void);
    fn porthole_record_av_writer_free_error(error: *mut c_char);
}

impl AvMovieWriter {
    pub fn new(settings: &MovieWriterSettings) -> Result<Self, ClientError> {
        validate_movie_writer_settings(settings).map_err(ClientError::Local)?;
        let path = settings
            .output
            .to_str()
            .ok_or_else(|| ClientError::Local("record output path must be valid UTF-8".to_string()))?;
        let path = CString::new(path.as_bytes()).map_err(|_| ClientError::Local("record output path contains a nul byte".to_string()))?;
        let mut writer = std::ptr::null_mut();
        call_av_writer(|error| {
            // SAFETY: path is a valid nul-terminated string and out pointers are valid for the call.
            unsafe { porthole_record_av_writer_create(path.as_ptr(), settings.width, settings.height, settings.stride, &mut writer, error) }
        })?;
        let raw = NonNull::new(writer).ok_or_else(|| ClientError::Local("AVAssetWriter returned a null writer".to_string()))?;
        Ok(Self {
            raw,
            finished: false,
            stride: settings.stride as usize,
            height: settings.height as usize,
        })
    }

    pub fn append(&mut self, timestamp_ns: u64, bytes: &[u8]) -> Result<(), ClientError> {
        let required_len = self
            .stride
            .checked_mul(self.height)
            .ok_or_else(|| ClientError::Local("record frame dimensions overflow".to_string()))?;
        if bytes.len() < required_len {
            return Err(ClientError::Local(format!(
                "record frame payload is shorter than stride * height: {} < {required_len}",
                bytes.len()
            )));
        }
        call_av_writer(|error| {
            // SAFETY: raw is owned by self and bytes points to required_len readable bytes for the duration of the call.
            unsafe { porthole_record_av_writer_append(self.raw.as_ptr(), timestamp_ns, bytes.as_ptr(), required_len, error) }
        })
    }

    pub fn finish(&mut self) -> Result<(), ClientError> {
        if self.finished {
            return Ok(());
        }
        call_av_writer(|error| {
            // SAFETY: raw is owned by self and the shim serially finishes the writer.
            unsafe { porthole_record_av_writer_finish(self.raw.as_ptr(), error) }
        })?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for AvMovieWriter {
    fn drop(&mut self) {
        // SAFETY: raw is created by porthole_record_av_writer_create and destroyed exactly once here.
        unsafe {
            porthole_record_av_writer_destroy(self.raw.as_ptr());
        }
    }
}

fn call_av_writer(call: impl FnOnce(*mut *mut c_char) -> c_int) -> Result<(), ClientError> {
    let mut error = std::ptr::null_mut();
    let status = call(&mut error);
    if status == 0 {
        return Ok(());
    }
    Err(ClientError::Local(take_error(error)))
}

fn take_error(error: *mut c_char) -> String {
    if error.is_null() {
        return "AVAssetWriter failed".to_string();
    }
    // SAFETY: the shim returns a nul-terminated string allocated for this caller.
    let message = unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned();
    // SAFETY: error was allocated by the shim and must be freed through the matching shim function.
    unsafe {
        porthole_record_av_writer_free_error(error);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::AvMovieWriter;
    use crate::commands::record::MovieWriterSettings;

    #[test]
    fn writes_tiny_quicktime_movie() {
        let tempdir = tempfile::tempdir().unwrap();
        let output = tempdir.path().join("tiny.mov");
        let settings = MovieWriterSettings {
            output: output.clone(),
            width: 2,
            height: 2,
            stride: 8,
            pixel_format: "bgra8_unorm".to_string(),
        };
        let mut writer = AvMovieWriter::new(&settings).unwrap();
        let black = vec![0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
        writer.append(0, &black).unwrap();
        writer.append(1_000_000, &black).unwrap();
        writer.finish().unwrap();

        assert!(std::fs::metadata(output).unwrap().len() > 0);
    }
}
