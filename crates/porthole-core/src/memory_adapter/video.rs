//! Canned video capture session used by both
//! [`MemoryAdapter::start_video_capture`] and `_publisher`.

use async_trait::async_trait;

use crate::{
    PortholeError,
    adapter::{
        VideoCaptureColorSpace, VideoCaptureDamageKind, VideoCaptureFrame, VideoCapturePixelFormat, VideoCaptureSession,
        VideoCaptureSyncKind, VideoCaptureTimestampClock,
    },
};

pub(super) struct FakeVideoSession {
    emitted: bool,
}

impl FakeVideoSession {
    pub(super) fn new() -> Self {
        Self { emitted: false }
    }

    /// Construct a session that has already fired its single frame elsewhere
    /// (e.g. via the publisher path) — `next_frame` returns `None` immediately.
    pub(super) fn exhausted() -> Self {
        Self { emitted: true }
    }
}

#[async_trait]
impl VideoCaptureSession for FakeVideoSession {
    async fn next_frame(&mut self) -> Result<Option<VideoCaptureFrame>, PortholeError> {
        if self.emitted {
            return Ok(None);
        }
        self.emitted = true;
        Ok(Some(canned_frame()))
    }
}

pub(super) fn canned_frame() -> VideoCaptureFrame {
    VideoCaptureFrame {
        sequence: 1,
        timestamp_ns: 123_456_789,
        timestamp_clock: VideoCaptureTimestampClock::UnixTime,
        width: 2,
        height: 1,
        stride: 8,
        pixel_format: VideoCapturePixelFormat::Bgra8Unorm,
        color_space: VideoCaptureColorSpace::Unknown,
        sync_kind: VideoCaptureSyncKind::CpuCopyComplete,
        damage_kind: VideoCaptureDamageKind::FullFrame,
        damage_base_sequence: 1,
        dropped_before_publish: 0,
        producer_drop_count: 0,
        bytes: vec![0, 64, 128, 255, 255, 64, 128, 255],
    }
}
