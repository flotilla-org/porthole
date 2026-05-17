use crate::{client::ClientError, commands::record::MovieWriterSettings};

pub struct AvMovieWriter;

impl AvMovieWriter {
    pub fn new(_settings: &MovieWriterSettings) -> Result<Self, ClientError> {
        Err(ClientError::Local("record movie writer is only supported on macOS".to_string()))
    }

    pub fn append(&mut self, _timestamp_ns: u64, _bytes: &[u8]) -> Result<(), ClientError> {
        Err(ClientError::Local("record movie writer is only supported on macOS".to_string()))
    }

    pub fn finish(&mut self) -> Result<(), ClientError> {
        Err(ClientError::Local("record movie writer is only supported on macOS".to_string()))
    }
}
