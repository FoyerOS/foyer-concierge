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
    /// Full systemd unit name, e.g. "foyer-concierge.service".
    pub name: String,
    pub description: String,
    /// systemd LoadState: "loaded", "not-found", "masked", ...
    pub load_state: String,
    /// systemd ActiveState: "active", "inactive", "failed", "activating", "deactivating".
    pub active_state: String,
    /// systemd SubState: "running", "dead", "exited", "failed", ...
    pub sub_state: String,
    /// Raw `GetUnitFileState` result ("enabled", "disabled", "static", "masked", ...);
    /// `None` if systemd couldn't report one.
    pub unit_file_state: Option<String>,
    /// Convenience: true iff `unit_file_state` starts with "enabled".
    pub enabled: bool,
    /// Convenience: true iff `active_state == "active"`.
    pub active: bool,
    /// Coarse status for a UI badge, derived server-side from `active_state`.
    pub health: ServiceHealth,
    /// Config files mapped to this unit (see the managed-service registry); empty if none.
    pub config_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceHealth {
    Ok,
    Failed,
    Transitioning,
    Inactive,
    Unknown,
}

/// A config file mapped to a managed service.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ServiceConfigFile {
    /// Absolute path on disk, for display only.
    pub path: String,
    pub content: String,
    /// Change-detection token (hash of `content`); an update must echo it back.
    pub etag: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct UpdateServiceConfigRequest {
    pub content: String,
    /// Must match the current file's `etag`, else 409 Conflict.
    pub etag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiskInfo {
    pub device: String,
    pub size_bytes: u64,
    pub model: Option<String>,
}

/// Current state of HTTPS termination at haproxy.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TlsStatus {
    pub enabled: bool,
    /// Domain the current leaf certificate was issued for.
    pub domain: Option<String>,
    /// True if the CA is Foyer-generated; false if a power user imported
    /// their own via the exit hatch (`PUT /tls/ca`).
    pub ca_managed: bool,
    /// RFC 3339 expiry of the root CA, if one has been generated/imported.
    pub ca_not_after: Option<String>,
    /// RFC 3339 expiry of the current leaf certificate, if TLS has ever been enabled.
    pub cert_not_after: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct EnableTlsRequest {
    pub domain: String,
}

/// Exit hatch: replace the CA concierge signs leaf certificates with.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SetCaRequest {
    pub ca_cert_pem: String,
    pub ca_key_pem: String,
}
