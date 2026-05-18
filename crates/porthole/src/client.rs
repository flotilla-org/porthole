use std::{
    path::Path,
    time::{Duration, Instant},
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, header::AUTHORIZATION};
use hyper_util::client::legacy::Client;
use hyperlocal::{UnixClientExt, UnixConnector, Uri as UnixUri};
use porthole_protocol::error::WireError;
use serde::{Serialize, de::DeserializeOwned};

pub struct DaemonClient {
    socket: std::path::PathBuf,
    http: Client<UnixConnector, Full<Bytes>>,
    bearer_token: Option<String>,
}

impl DaemonClient {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
            http: Client::unix(),
            bearer_token: None,
        }
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let req = self.build_empty_request(Method::GET, path)?;
        self.send_and_parse(req).await
    }

    pub async fn post_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T, ClientError> {
        let req = self.build_json_request(Method::POST, path, body)?;
        self.send_and_parse(req).await
    }

    pub async fn delete_empty(&self, path: &str) -> Result<(), ClientError> {
        let req = self.build_empty_request(Method::DELETE, path)?;
        let res = self.http.request(req).await?;
        let status = res.status();
        let body = res.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            let wire: WireError = serde_json::from_slice(&body).map_err(ClientError::from)?;
            return Err(ClientError::Api(wire));
        }
        Ok(())
    }

    fn build_empty_request(&self, method: Method, path: &str) -> Result<Request<Full<Bytes>>, hyper::http::Error> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = &self.bearer_token {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(Full::new(Bytes::new()))
    }

    fn build_json_request<B: Serialize>(&self, method: Method, path: &str, body: &B) -> Result<Request<Full<Bytes>>, ClientError> {
        let uri: hyper::Uri = UnixUri::new(&self.socket, path).into();
        let body_bytes = serde_json::to_vec(body)?;
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = &self.bearer_token {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        Ok(builder.body(Full::new(Bytes::from(body_bytes)))?)
    }

    /// Block until /info responds successfully, with exponential backoff up to
    /// `timeout`. Used after a daemon restart (kickstart) — the UDS socket
    /// briefly disappears while launchd brings the new process up.
    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<(), ClientError> {
        use porthole_protocol::info::InfoResponse;
        let deadline = Instant::now() + timeout;
        let mut delay = Duration::from_millis(100);
        let mut last_err: Option<ClientError> = None;
        while Instant::now() < deadline {
            match self.get_json::<InfoResponse>("/info").await {
                Ok(_) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
            // Clamp the sleep to the remaining budget so we exit on time
            // rather than overshooting by up to `delay` (which can grow to 2s).
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(delay.min(remaining)).await;
            delay = (delay * 2).min(Duration::from_millis(2000));
        }
        Err(last_err.unwrap_or_else(|| ClientError::Local("daemon did not respond before timeout".into())))
    }

    async fn send_and_parse<T: DeserializeOwned>(&self, req: Request<Full<Bytes>>) -> Result<T, ClientError> {
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

#[cfg(test)]
mod tests {
    use hyper::header::AUTHORIZATION;

    use super::*;

    #[test]
    fn client_request_omits_bearer_token_by_default() {
        let client = DaemonClient::new("/tmp/porthole.sock");

        let request = client.build_empty_request(Method::GET, "/info").unwrap();

        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn client_request_adds_bearer_token_when_configured() {
        let client = DaemonClient::new("/tmp/porthole.sock").with_bearer_token("pta_agent.secret");

        let request = client.build_empty_request(Method::GET, "/info").unwrap();

        assert_eq!(request.headers().get(AUTHORIZATION).unwrap(), "Bearer pta_agent.secret");
    }
}
