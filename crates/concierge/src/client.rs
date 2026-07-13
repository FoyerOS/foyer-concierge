//! HTTP client used by the CLI subcommands to talk to the daemon over its
//! unix socket. The socket is root/admin-group only, so no session auth is
//! needed on this transport.

use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow};
use concierge_api::ApiErrorBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::client::legacy::Client as HyperClient;
use hyperlocal::{UnixClientExt, UnixConnector, Uri};
use serde::de::DeserializeOwned;

pub struct Client {
    socket: PathBuf,
    http: HyperClient<UnixConnector, Full<Bytes>>,
}

/// The daemon rejected the request; carries the API error envelope.
#[derive(Debug, thiserror::Error)]
#[error("{}: {}", .status, .body.message)]
pub struct ApiError {
    pub status: hyper::StatusCode,
    pub body: ApiErrorBody,
}

impl Client {
    pub fn new(socket: &Path) -> Self {
        Self {
            socket: socket.to_owned(),
            http: HyperClient::unix(),
        }
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let uri: hyper::Uri = Uri::new(&self.socket, path).into();
        let response = self.http.get(uri).await.with_context(|| {
            format!(
                "cannot reach the concierge daemon on {} (is it running?)",
                self.socket.display()
            )
        })?;

        let status = response.status();
        let bytes = response.into_body().collect().await?.to_bytes();
        if !status.is_success() {
            let body: ApiErrorBody = serde_json::from_slice(&bytes).unwrap_or(ApiErrorBody {
                code: "unknown".into(),
                message: String::from_utf8_lossy(&bytes).into_owned(),
            });
            return Err(anyhow!(ApiError { status, body }));
        }
        serde_json::from_slice(&bytes).context("invalid JSON from daemon")
    }
}
