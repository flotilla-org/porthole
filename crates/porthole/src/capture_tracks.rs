//! Consumer side of Jackstay's track handshake (v3) on a capture connection.
//!
//! Once portholed has authorized a capture connection (the CPU session
//! preface, or a Windows native attach), it offers the session's tracks: one
//! video track. The consumer selects it, and the returned media stream then
//! carries Jackstay's CPU or D3D11 setup.

use jackstay::{
    handshake::{self, Kind, Selected},
    input::Mode,
    local::Stream,
};

/// Select the offered video track, required, and return the media stream.
pub fn select_video(stream: Stream) -> Result<Stream, handshake::Error> {
    let offered = handshake::connect(stream)?;
    let video = offered
        .offer()
        .tracks
        .iter()
        .find(|track| track.kind == Kind::Video)
        .ok_or(handshake::Error::Protocol("no video track offered"))?
        .id;
    let selection = [Selected {
        id: video,
        required: true,
        write: false,
    }];
    // No input track is selected, so the input mode is never used.
    Ok(offered.select(&selection, Mode::Cooperative)?.media)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use cpu::{CpuConnectError, connect_cpu};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod cpu {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixStream,
        time::Duration,
    };

    use jackstay::{
        acquisition::socket::{CpuSetupClient, SocketError},
        daemon::{ConnectedSession, SessionInfo},
        handshake,
    };

    const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_REPLY: usize = 16 * 1024;

    #[derive(Debug, thiserror::Error)]
    pub enum CpuConnectError {
        #[error("CPU capture session I/O: {0}")]
        Io(#[from] std::io::Error),
        #[error("CPU capture session reply: {0}")]
        Reply(String),
        #[error("CPU capture session rejected: {0}")]
        Rejected(String),
        #[error("CPU capture track selection: {0}")]
        Handshake(#[from] handshake::Error),
        #[error("CPU capture setup: {0}")]
        Setup(#[from] SocketError),
    }

    #[derive(serde::Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum Reply {
        CpuOpened,
        Rejected { message: String },
    }

    /// Open a portholed CPU capture session: present the session, track and
    /// token, select the video track, then admit a consumer that may hold
    /// `holding` frames at once.
    ///
    /// # Safety
    /// The daemon must be a trusted conforming producer, and this process
    /// remains the sole recipient of the process-bound mappings: do not fork,
    /// forward or replay them.
    pub unsafe fn connect_cpu(info: SessionInfo, holding: u32) -> Result<ConnectedSession, CpuConnectError> {
        let mut stream = UnixStream::connect(&info.fd_socket_path)?;
        stream.set_read_timeout(Some(SETUP_TIMEOUT))?;
        stream.set_write_timeout(Some(SETUP_TIMEOUT))?;
        let request = serde_json::json!({
            "op": "open_cpu_acquisition", "session_id": info.session_id,
            "track_id": info.track_id, "bearer_token": info.bearer_token,
        });
        writeln!(stream, "{request}")?;
        // One byte at a time: the track handshake follows the newline.
        let mut reply = Vec::new();
        loop {
            if reply.len() == MAX_REPLY {
                return Err(CpuConnectError::Reply("reply is too large".to_owned()));
            }
            let mut byte = [0];
            stream.read_exact(&mut byte)?;
            if byte == *b"\n" {
                break;
            }
            reply.push(byte[0]);
        }
        match serde_json::from_slice(&reply).map_err(|error| CpuConnectError::Reply(error.to_string()))? {
            Reply::CpuOpened => {}
            Reply::Rejected { message } => return Err(CpuConnectError::Rejected(message)),
        }
        let media = super::select_video(stream)?;
        // SAFETY: delegated to this function's sole-producer/process contract.
        let mut setup = unsafe { CpuSetupClient::from_stream(media) };
        let consumer = setup.attach(holding)?;
        Ok(ConnectedSession { setup, consumer, info })
    }
}
