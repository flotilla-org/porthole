#[cfg(unix)]
use std::path::{Path, PathBuf};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use axum::Router;
use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response, Uri, body::Incoming};
use hyper_util::{
    client::legacy::{Client, connect::Connection},
    rt::{TokioExecutor, TokioIo},
};
use tower::Service;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(windows)]
    WindowsNamedPipe(String),
}

impl Endpoint {
    #[cfg(unix)]
    #[must_use]
    pub fn from_socket_path(path: PathBuf) -> Self {
        Self::Unix(path)
    }

    #[cfg(windows)]
    #[must_use]
    pub fn from_named_pipe_name(name: impl Into<String>) -> Self {
        Self::WindowsNamedPipe(name.into())
    }

    #[cfg(unix)]
    #[must_use]
    pub fn as_socket_path(&self) -> &Path {
        match self {
            Self::Unix(path) => path,
        }
    }

    #[cfg(windows)]
    #[must_use]
    pub fn as_named_pipe_name(&self) -> &str {
        match self {
            Self::WindowsNamedPipe(name) => name,
        }
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => path.display().to_string(),
            #[cfg(windows)]
            Self::WindowsNamedPipe(name) => name.clone(),
        }
    }
}

#[must_use]
pub fn control_endpoint() -> Endpoint {
    #[cfg(unix)]
    {
        Endpoint::Unix(socket_path())
    }
    #[cfg(windows)]
    {
        Endpoint::WindowsNamedPipe(named_pipe_name())
    }
}

#[cfg(unix)]
fn socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("PORTHOLE_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole.sock");
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("porthole").join("porthole.sock");
    }
    if let Ok(tmp) = std::env::var("TMPDIR") {
        let uid = unsafe { libc_getuid() };
        return PathBuf::from(tmp).join(format!("porthole-{uid}")).join("porthole.sock");
    }
    let uid = unsafe { libc_getuid() };
    PathBuf::from("/tmp").join(format!("porthole-{uid}")).join("porthole.sock")
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[cfg(windows)]
fn named_pipe_name() -> String {
    // SECURITY: confidentiality currently relies on the pipe's default security
    // descriptor from ServerOptions::create. On a shared / RDP host this must
    // restrict cross-user access the way the Unix socket's per-uid runtime dir
    // (0700) does; the default DACL grants the creator + SYSTEM/Administrators,
    // but this needs an explicit confirmation (and likely a hardened DACL)
    // before multi-user Windows deployment. Tracked as Windows-port follow-up.
    let Some(username) = std::env::var("USERNAME").ok().filter(|value| !value.is_empty()) else {
        return r"\\.\pipe\porthole".to_string();
    };
    format!(r"\\.\pipe\porthole-{username}")
}

#[derive(Clone, Debug)]
pub struct LocalConnector {
    endpoint: Endpoint,
}

impl LocalConnector {
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }
}

impl Service<Uri> for LocalConnector {
    type Response = LocalStream;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: Uri) -> Self::Future {
        let endpoint = self.endpoint.clone();
        Box::pin(async move {
            match endpoint {
                #[cfg(unix)]
                Endpoint::Unix(path) => Ok(LocalStream::Unix(TokioIo::new(tokio::net::UnixStream::connect(path).await?))),
                #[cfg(windows)]
                Endpoint::WindowsNamedPipe(name) => {
                    use tokio::net::windows::named_pipe::ClientOptions;

                    let pipe = tokio::task::spawn_blocking(move || ClientOptions::new().open(name))
                        .await
                        .map_err(std::io::Error::other)??;
                    Ok(LocalStream::WindowsNamedPipe(TokioIo::new(pipe)))
                }
            }
        })
    }
}

pub enum LocalStream {
    #[cfg(unix)]
    Unix(TokioIo<tokio::net::UnixStream>),
    #[cfg(windows)]
    WindowsNamedPipe(TokioIo<tokio::net::windows::named_pipe::NamedPipeClient>),
}

impl hyper::rt::Read for LocalStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: hyper::rt::ReadBufCursor<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => hyper::rt::Read::poll_read(Pin::new(stream), cx, buf),
            #[cfg(windows)]
            Self::WindowsNamedPipe(stream) => hyper::rt::Read::poll_read(Pin::new(stream), cx, buf),
        }
    }
}

impl hyper::rt::Write for LocalStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => hyper::rt::Write::poll_write(Pin::new(stream), cx, buf),
            #[cfg(windows)]
            Self::WindowsNamedPipe(stream) => hyper::rt::Write::poll_write(Pin::new(stream), cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => hyper::rt::Write::poll_flush(Pin::new(stream), cx),
            #[cfg(windows)]
            Self::WindowsNamedPipe(stream) => hyper::rt::Write::poll_flush(Pin::new(stream), cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => hyper::rt::Write::poll_shutdown(Pin::new(stream), cx),
            #[cfg(windows)]
            Self::WindowsNamedPipe(stream) => hyper::rt::Write::poll_shutdown(Pin::new(stream), cx),
        }
    }

    fn is_write_vectored(&self) -> bool {
        false
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        match self.get_mut() {
            #[cfg(unix)]
            Self::Unix(stream) => hyper::rt::Write::poll_write_vectored(Pin::new(stream), cx, bufs),
            #[cfg(windows)]
            Self::WindowsNamedPipe(stream) => hyper::rt::Write::poll_write_vectored(Pin::new(stream), cx, bufs),
        }
    }
}

impl Connection for LocalStream {
    fn connected(&self) -> hyper_util::client::legacy::connect::Connected {
        hyper_util::client::legacy::connect::Connected::new()
    }
}

#[derive(Clone)]
pub struct LocalHttpClient {
    http: Client<LocalConnector, Full<Bytes>>,
}

impl LocalHttpClient {
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            http: Client::builder(TokioExecutor::new()).build(LocalConnector::new(endpoint)),
        }
    }

    pub async fn request(&self, request: Request<Full<Bytes>>) -> Result<Response<Incoming>, hyper_util::client::legacy::Error> {
        self.http.request(request).await
    }
}

pub async fn serve(endpoint: Endpoint, router: Router) -> std::io::Result<()> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(socket_path) => serve_unix(socket_path, router).await,
        #[cfg(windows)]
        Endpoint::WindowsNamedPipe(name) => serve_windows(name, router).await,
    }
}

#[cfg(unix)]
async fn serve_unix(socket_path: PathBuf, router: Router) -> std::io::Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }
    let listener = tokio::net::UnixListener::bind(&socket_path)?;
    axum::serve(listener, router).await
}

#[cfg(windows)]
async fn serve_windows(name: String, router: Router) -> std::io::Result<()> {
    use hyper_util::{server::conn::auto, service::TowerToHyperService};
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    fn create_pipe(name: &str, first_pipe_instance: bool) -> std::io::Result<NamedPipeServer> {
        ServerOptions::new().first_pipe_instance(first_pipe_instance).create(name)
    }

    // Keep re-creating the next pipe instance until it succeeds. A transient
    // failure to re-arm (or to accept a connection) must degrade to a dropped
    // connection, not propagate out of the accept loop and take the whole
    // daemon down — matching how axum::serve's Unix accept loop tolerates
    // transient per-connection errors and keeps serving.
    async fn rearm(name: &str) -> NamedPipeServer {
        loop {
            match create_pipe(name, false) {
                Ok(pipe) => return pipe,
                Err(error) => {
                    tracing::error!(%error, "failed to re-arm named pipe instance; retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    // The first instance must bind or there is no endpoint at all (mirrors a
    // failed UnixListener::bind, which the Unix path also propagates).
    let mut pipe = create_pipe(&name, true)?;
    loop {
        if let Err(error) = pipe.connect().await {
            tracing::warn!(%error, "named pipe connect failed; re-arming instance");
            pipe = rearm(&name).await;
            continue;
        }
        let connected = pipe;
        pipe = rearm(&name).await;
        let service = TowerToHyperService::new(router.clone().into_service());
        tokio::spawn(async move {
            let io = TokioIo::new(connected);
            let builder = auto::Builder::new(TokioExecutor::new());
            if let Err(error) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::debug!(%error, "named pipe HTTP connection ended with error");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::path::PathBuf;

    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    #[test]
    fn porthole_runtime_dir_wins() {
        unsafe {
            std::env::set_var("PORTHOLE_RUNTIME_DIR", "/tmp/test-porthole");
        }
        let p = control_endpoint();
        assert_eq!(p, Endpoint::Unix(PathBuf::from("/tmp/test-porthole/porthole.sock")));
        unsafe {
            std::env::remove_var("PORTHOLE_RUNTIME_DIR");
        }
    }
}
