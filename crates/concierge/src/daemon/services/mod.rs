//! Service layer: one trait per management domain. The HTTP handlers and
//! (eventually) other frontends only ever talk to these traits, so feature
//! work happens here without touching the transport code.

pub mod storage;
pub mod system;
pub mod units;
pub mod users;

use async_trait::async_trait;
use concierge_api::{DiskInfo, ServiceInfo, SystemStatus, UserInfo};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("not implemented yet")]
    Unimplemented,
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
}

#[async_trait]
pub trait StorageService: Send + Sync {
    async fn disks(&self) -> Result<Vec<DiskInfo>>;
}
