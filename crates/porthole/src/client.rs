use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri as UnixUri};
use porthole_protocol::error::WireError;
use serde::de::DeserializeOwned;
use serde::Serialize;

pub struct DaemonClient {
    socket: std::path::PathBuf,
    http: Client<UnixConnector, Full<Bytes>>,
}

impl DaemonClient {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
            http: Client::unix(),
        }
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let req = Request::builder().method(Method::GET).uri(uri).body(Full::new(Bytes::new()))?;
        self.send_and_parse(req).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ClientError> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let body_bytes = serde_json::to_vec(body)?;
        let req = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body_bytes)))?;
        self.send_and_parse(req).await
    }

    async fn send_and_parse<T: DeserializeOwned>(
        &self,
        req: Request<Full<Bytes>>,
    ) -> Result<T, ClientError> {
        let res = self.http.request(req).await?;
        let status = res.status();
        let body = res.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            let wire: WireError = serde_json::from_slice(&body).map_err(ClientError::from)?;
            return Err(ClientError::Api(wire));
        }
        let value = serde_json::from_slice(&body)?;
        Ok(value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http: {0}")]
    Http(#[from] hyper::Error),
    #[error("http legacy: {0}")]
    HttpLegacy(#[from] hyper_util::client::legacy::Error),
    #[error("request build: {0}")]
    RequestBuild(#[from] hyper::http::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("api: {} ({})", .0.code, .0.message)]
    Api(WireError),
    #[error("{0}")]
    Local(String),
}
