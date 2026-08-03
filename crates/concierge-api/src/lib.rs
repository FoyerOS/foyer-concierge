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

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub username: String,
    pub current_password: String,
    pub new_password: String,
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

/// A block device as seen from sysfs, with a role telling the caller whether
/// it is safe to add to the `/data` pool.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiskInfo {
    /// e.g. "/dev/sdb".
    pub path: String,
    /// Kernel device name, e.g. "sdb".
    pub kname: String,
    pub size_bytes: u64,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// Inferred from the kname/subsystem link: "nvme", "ata", "usb", "mmc", or "unknown".
    pub transport: String,
    pub rotational: bool,
    pub removable: bool,
    pub role: DiskRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiskRole {
    /// Carries the ESP or the `foyer-data` partition; never addable to the pool.
    System,
    /// Already a member of the `/data` btrfs pool.
    PoolMember,
    /// No partition table, filesystem signature or mounted partition; safe to add.
    Available,
    /// Has a partition table, a filesystem signature or a mounted partition,
    /// but is neither the system disk nor a pool member.
    InUse,
}

/// Status of the `/data` btrfs pool.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolStatus {
    /// btrfs filesystem UUID.
    pub uuid: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub free_bytes: u64,
    pub devices: Vec<PoolDevice>,
    /// True if any member device is missing.
    pub degraded: bool,
    /// The add/remove operation currently running, if any.
    pub operation: Option<PoolOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolDevice {
    pub devid: u64,
    pub path: String,
    pub size_bytes: u64,
    pub used_bytes: u64,
    pub missing: bool,
}

/// A long-running pool mutation (`btrfs device add`/`remove`), tracked so
/// `PoolStatus` can report progress instead of blocking the request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PoolOperation {
    pub kind: PoolOperationKind,
    /// Device path the operation was started with.
    pub device: String,
    /// RFC 3339 timestamp.
    pub started_at: String,
    pub state: PoolOperationState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolOperationKind {
    AddDevice,
    RemoveDevice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PoolOperationState {
    Running,
    Succeeded,
    Failed { message: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct AddDiskRequest {
    pub device: String,
    /// Force-add a disk that isn't `Available` (e.g. carries a stale
    /// filesystem signature). Never overrides `DiskRole::System`.
    pub wipe: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct RemoveDiskRequest {
    pub device: String,
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
