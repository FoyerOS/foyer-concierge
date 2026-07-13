//! Shared API types for foyer-concierge.
//!
//! Everything that crosses the wire between the daemon, the CLI and the
//! WebGUI lives here, so all three always agree on the schema.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Mounted under `/api/{API_VERSION}/...`.
pub const API_VERSION: &str = "v1";

/// Error envelope returned by every non-2xx API response.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorBody {
    /// Stable machine-readable code, e.g. `unauthorized`, `unimplemented`.
    pub code: String,
    /// Human-readable description.
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
    /// Daemon version (crate version of the running binary).
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SystemStatus {
    pub hostname: String,
    /// Seconds since boot.
    pub uptime_secs: u64,
    /// 1, 5 and 15 minute load averages.
    pub load_avg: [f64; 3],
    pub memory: MemoryStatus,
    /// Version reported by systemd over D-Bus, if reachable.
    pub systemd_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MemoryStatus {
    pub total_kib: u64,
    pub available_kib: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionInfo {
    pub username: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserInfo {
    pub name: String,
    pub uid: u32,
}

/// A managed service (systemd unit, possibly podman/Quadlet-backed).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceInfo {
    pub name: String,
    pub enabled: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiskInfo {
    pub device: String,
    pub size_bytes: u64,
    pub model: Option<String>,
}
