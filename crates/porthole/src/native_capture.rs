//! Consumer side of a Windows native capture session's attach endpoint.
//!
//! `POST /capture-sessions/surfaces/{id}?native=true` on Windows returns a
//! [`NativeCaptureInfo`] with transport
//! [`NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT`]: a Jackstay Local
//! Endpoint name in `Session` scope and a per-session attach token. [`open`]
//! connects (verifying the server is this user's process in this logon
//! session), presents the token and returns the stream, which then carries
//! Jackstay's D3D11 setup (`D3d11SetupClient::from_stream`) or CPU setup
//! (`CpuSetupClient::from_stream`), as the returned publication says.

use std::io::{Read, Write};

use jackstay::local::{self, Endpoint, Scope, Stream, Transport};
use porthole_protocol::capture_sessions::{
    NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT, NativeCaptureInfo, WindowsNativeAttachReply, WindowsNativeAttachRequest,
    WindowsNativePublication,
};

/// Why a native attach did not open.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("not a Windows native capture endpoint (transport {0})")]
    Transport(u32),
    #[error("native attach endpoint: {0}")]
    Endpoint(#[from] local::Error),
    #[error("native attach I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("native attach protocol: {0}")]
    Protocol(String),
    /// The session refused: wrong token, closed or failed session, or the
    /// connection limit. The message is the daemon's.
    #[error("native attach rejected: {0}")]
    Rejected(String),
}

/// An opened attach connection, ready for Jackstay setup.
#[derive(Debug)]
pub struct Opened {
    pub publication: WindowsNativePublication,
    /// Advances when device loss replaces the session's publication.
    pub epoch: u64,
    pub stream: Stream,
}

/// Connect to a Windows native capture session and present its token.
pub fn open(session_id: &str, native: &NativeCaptureInfo) -> Result<Opened, OpenError> {
    if native.transport_kind != NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT {
        return Err(OpenError::Transport(native.transport_kind));
    }
    let endpoint = Endpoint::new(Scope::Session, &native.endpoint, Transport::LocalStream)?;
    let mut stream = local::connect(&endpoint)?.into_stream();
    let mut line = serde_json::to_vec(&WindowsNativeAttachRequest::OpenNativeCapture {
        session_id: session_id.to_owned(),
        attach_token: native.attach_token.clone(),
    })
    .map_err(|error| OpenError::Protocol(error.to_string()))?;
    line.push(b'\n');
    stream.write_all(&line)?;
    stream.flush()?;
    // One byte at a time: Jackstay setup bytes follow the newline.
    let mut reply = Vec::new();
    loop {
        if reply.len() == 16 * 1024 {
            return Err(OpenError::Protocol("attach reply is too large".to_owned()));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        if byte == *b"\n" {
            break;
        }
        reply.push(byte[0]);
    }
    match serde_json::from_slice(&reply).map_err(|error| OpenError::Protocol(error.to_string()))? {
        WindowsNativeAttachReply::NativeCaptureOpened { publication, epoch } => Ok(Opened {
            publication,
            epoch,
            stream,
        }),
        WindowsNativeAttachReply::Rejected { message } => Err(OpenError::Rejected(message)),
    }
}
