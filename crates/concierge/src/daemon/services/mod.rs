//! Service layer: one trait per management domain. The HTTP handlers and
//! (eventually) other frontends only ever talk to these traits, so feature
//! work happens here without touching the transport code.

pub mod managed_services;
pub mod routes;
pub mod storage;
pub mod system;
pub mod tls;
pub mod units;
pub mod users;

use async_trait::async_trait;
use concierge_api::{DiskInfo, ServiceConfigFile, ServiceInfo, SystemStatus, TlsStatus, UserInfo};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("not implemented yet")]
    Unimplemented,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, ServiceError>;

#[async_trait]
pub trait SystemService: Send + Sync {
    async fn status(&self) -> Result<SystemStatus>;
}

#[async_trait]
pub trait UserService: Send + Sync {
    async fn list(&self) -> Result<Vec<UserInfo>>;
}

/// Managed daemons: systemd units, including podman containers via Quadlet.
#[async_trait]
pub trait UnitService: Send + Sync {
    async fn list(&self) -> Result<Vec<ServiceInfo>>;
    async fn enable(&self, unit: &str) -> Result<ServiceInfo>;
    async fn disable(&self, unit: &str) -> Result<ServiceInfo>;
    /// Ask systemd to reload the unit (`ExecReload`), not re-run `daemon-reload`.
    async fn reload(&self, unit: &str) -> Result<ServiceInfo>;
    async fn get_config(&self, unit: &str, path: &str) -> Result<ServiceConfigFile>;
    async fn set_config(
        &self,
        unit: &str,
        path: &str,
        content: String,
        etag: &str,
    ) -> Result<ServiceConfigFile>;
}

#[async_trait]
pub trait StorageService: Send + Sync {
    async fn disks(&self) -> Result<Vec<DiskInfo>>;
}

/// HTTPS termination at haproxy: a self-signed CA concierge owns, a leaf
/// certificate issued for the configured domain, and the haproxy.cfg
/// rewrite/reload that wires the leaf cert in.
#[async_trait]
pub trait TlsService: Send + Sync {
    async fn status(&self) -> Result<TlsStatus>;
    async fn enable(&self, domain: &str) -> Result<TlsStatus>;
    async fn disable(&self) -> Result<TlsStatus>;
    /// PEM of the current root CA certificate (not the key).
    async fn ca_cert(&self) -> Result<String>;
    /// Exit hatch: replace the CA used to sign the leaf certificate, e.g.
    /// with a power user's own internal CA instead of Foyer's.
    async fn set_ca(&self, ca_cert_pem: String, ca_key_pem: String) -> Result<TlsStatus>;
    /// Recompute which `routes::ROUTABLE_SERVICES` are currently enabled
    /// and re-route haproxy accordingly; a no-op while TLS is off, since
    /// plain-HTTP mode never routes by subdomain. Call after any routable
    /// service's enabled state changes.
    async fn sync_routes(&self) -> Result<()>;
}
