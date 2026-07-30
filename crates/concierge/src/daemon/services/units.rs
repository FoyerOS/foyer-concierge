use async_trait::async_trait;
use concierge_api::{ServiceConfigFile, ServiceHealth, ServiceInfo};

use super::managed_services::{self, ManagedService};
use super::{Result, ServiceError, UnitService};

/// Proxy to the systemd manager on the system bus.
#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait SystemdManager {
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    /// Loads (if necessary) and returns the unit's object path. Unlike
    /// `GetUnit`, this also returns units that exist on disk but haven't
    /// been referenced by anything yet.
    #[zbus(name = "LoadUnit")]
    fn load_unit(&self, name: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;

    #[zbus(name = "GetUnitFileState")]
    fn get_unit_file_state(&self, file: &str) -> zbus::Result<String>;

    #[zbus(name = "EnableUnitFiles")]
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<(String, String, String)>)>;

    #[zbus(name = "DisableUnitFiles")]
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<(String, String, String)>>;

    #[zbus(name = "Reload")]
    fn reload(&self) -> zbus::Result<()>;
}

/// Proxy to a single unit object, bound dynamically to whatever path
/// `LoadUnit`/`GetUnit` returns (there is no fixed default_path here).
#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
pub trait SystemdUnit {
    #[zbus(property, name = "Description")]
    fn description(&self) -> zbus::Result<String>;

    #[zbus(property, name = "LoadState")]
    fn load_state(&self) -> zbus::Result<String>;

    #[zbus(property, name = "ActiveState")]
    fn active_state(&self) -> zbus::Result<String>;

    #[zbus(property, name = "SubState")]
    fn sub_state(&self) -> zbus::Result<String>;
}

/// Unit management backed by systemd over D-Bus.
pub struct SystemdUnitService {
    connection: Option<zbus::Connection>,
}

impl SystemdUnitService {
    pub fn new(connection: Option<zbus::Connection>) -> Self {
        Self { connection }
    }

    /// Fetch the current state of `unit` over D-Bus. `config_paths` is left
    /// empty; callers fill it in from the managed-service registry.
    async fn query_unit(
        &self,
        connection: &zbus::Connection,
        unit: &str,
    ) -> zbus::Result<ServiceInfo> {
        let manager = SystemdManagerProxy::new(connection).await?;
        let object_path = manager.load_unit(unit).await?;
        let unit_proxy = SystemdUnitProxy::builder(connection)
            .path(object_path)?
            .build()
            .await?;

        let description = unit_proxy.description().await?;
        let load_state = unit_proxy.load_state().await?;
        let active_state = unit_proxy.active_state().await?;
        let sub_state = unit_proxy.sub_state().await?;
        let unit_file_state = manager.get_unit_file_state(unit).await.ok();

        let enabled = unit_file_state
            .as_deref()
            .is_some_and(|state| state.starts_with("enabled"));
        let active = active_state == "active";
        let health = health_for(&load_state, &active_state);

        Ok(ServiceInfo {
            name: unit.to_owned(),
            description,
            load_state,
            active_state,
            sub_state,
            unit_file_state,
            enabled,
            active,
            health,
            config_paths: Vec::new(),
        })
    }

    /// Describe a managed unit, degrading to an "unknown" placeholder
    /// (rather than failing the whole list) if D-Bus is unreachable. A unit
    /// that's simply missing from this build doesn't hit this path -
    /// systemd's LoadUnit succeeds regardless and reports LoadState
    /// "not-found", which `health_for` turns into `ServiceHealth::Unknown`.
    async fn describe_managed(&self, entry: &ManagedService) -> ServiceInfo {
        let config_paths = owned_config_paths(entry);
        let Some(connection) = &self.connection else {
            return unknown_service(entry.unit, config_paths);
        };
        match self.query_unit(connection, entry.unit).await {
            Ok(mut info) => {
                info.config_paths = config_paths;
                info
            }
            Err(error) => {
                tracing::warn!(unit = entry.unit, %error, "cannot query unit over D-Bus");
                unknown_service(entry.unit, config_paths)
            }
        }
    }

    fn connection(&self) -> Result<&zbus::Connection> {
        self.connection
            .as_ref()
            .ok_or_else(|| ServiceError::Other(anyhow::anyhow!("system D-Bus unavailable")))
    }

    async fn set_enabled(&self, unit: &str, enabled: bool) -> Result<ServiceInfo> {
        let entry = managed_services::find(unit)
            .ok_or_else(|| ServiceError::NotFound(unit.to_owned()))?;
        let connection = self.connection()?;
        let manager = SystemdManagerProxy::new(connection)
            .await
            .map_err(|error| map_dbus_error(unit, error))?;

        if enabled {
            manager
                .enable_unit_files(&[unit], false, false)
                .await
                .map_err(|error| map_dbus_error(unit, error))?;
        } else {
            manager
                .disable_unit_files(&[unit], false)
                .await
                .map_err(|error| map_dbus_error(unit, error))?;
        }
        manager
            .reload()
            .await
            .map_err(|error| map_dbus_error(unit, error))?;

        let mut info = self
            .query_unit(connection, unit)
            .await
            .map_err(|error| map_dbus_error(unit, error))?;
        info.config_paths = owned_config_paths(entry);
        Ok(info)
    }
}

#[async_trait]
impl UnitService for SystemdUnitService {
    async fn list(&self) -> Result<Vec<ServiceInfo>> {
        let mut infos = Vec::with_capacity(managed_services::MANAGED_SERVICES.len());
        for entry in managed_services::MANAGED_SERVICES {
            infos.push(self.describe_managed(entry).await);
        }
        Ok(infos)
    }

    async fn enable(&self, unit: &str) -> Result<ServiceInfo> {
        self.set_enabled(unit, true).await
    }

    async fn disable(&self, unit: &str) -> Result<ServiceInfo> {
        self.set_enabled(unit, false).await
    }

    async fn get_config(&self, unit: &str, path: &str) -> Result<ServiceConfigFile> {
        let config_path = resolve_config_path(unit, path)?;
        let content = std::fs::read_to_string(config_path)
            .map_err(|error| ServiceError::Other(error.into()))?;
        let etag = etag_for(&content);
        Ok(ServiceConfigFile {
            path: config_path.to_owned(),
            content,
            etag,
        })
    }

    async fn set_config(
        &self,
        unit: &str,
        path: &str,
        content: String,
        etag: &str,
    ) -> Result<ServiceConfigFile> {
        let config_path = resolve_config_path(unit, path)?;
        let path = std::path::Path::new(config_path);

        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| ServiceError::Other(error.into()))?;
        if !metadata.is_file() {
            return Err(ServiceError::Other(anyhow::anyhow!(
                "{config_path} is not a regular file"
            )));
        }

        let current = std::fs::read_to_string(path).map_err(|error| ServiceError::Other(error.into()))?;
        if etag_for(&current) != etag {
            return Err(ServiceError::Conflict(config_path.to_owned()));
        }

        let parent = path.parent().unwrap_or(std::path::Path::new("/"));
        let temp_path = parent.join(format!(
            ".{}.tmp",
            path.file_name().and_then(|name| name.to_str()).unwrap_or("concierge-config")
        ));
        std::fs::write(&temp_path, &content).map_err(|error| ServiceError::Other(error.into()))?;
        std::fs::rename(&temp_path, path).map_err(|error| ServiceError::Other(error.into()))?;

        let new_etag = etag_for(&content);
        Ok(ServiceConfigFile {
            path: config_path.to_owned(),
            content,
            etag: new_etag,
        })
    }
}

fn owned_config_paths(entry: &ManagedService) -> Vec<String> {
    entry.config_paths.iter().map(|path| (*path).to_owned()).collect()
}

fn unknown_service(unit: &str, config_paths: Vec<String>) -> ServiceInfo {
    ServiceInfo {
        name: unit.to_owned(),
        description: String::new(),
        load_state: "unknown".to_owned(),
        active_state: "unknown".to_owned(),
        sub_state: "unknown".to_owned(),
        unit_file_state: None,
        enabled: false,
        active: false,
        health: ServiceHealth::Unknown,
        config_paths,
    }
}

fn health_for(load_state: &str, active_state: &str) -> ServiceHealth {
    // systemd's LoadUnit doesn't error for a nonexistent unit; it returns a
    // stub with LoadState "not-found"/"masked" and ActiveState "inactive",
    // which would otherwise be indistinguishable from a real, installed but
    // stopped service.
    if load_state == "not-found" || load_state == "masked" {
        return ServiceHealth::Unknown;
    }
    match active_state {
        "active" => ServiceHealth::Ok,
        "failed" => ServiceHealth::Failed,
        "activating" | "deactivating" | "reloading" => ServiceHealth::Transitioning,
        "inactive" => ServiceHealth::Inactive,
        _ => ServiceHealth::Unknown,
    }
}

/// Look up `unit` in the managed-service registry and require `path` to
/// exactly match one of its config paths, returning the *static* string
/// from the registry (not the caller-supplied one) for the actual
/// filesystem access.
fn resolve_config_path(unit: &str, path: &str) -> Result<&'static str> {
    let entry =
        managed_services::find(unit).ok_or_else(|| ServiceError::NotFound(unit.to_owned()))?;
    entry
        .config_paths
        .iter()
        .find(|candidate| **candidate == path)
        .copied()
        .ok_or_else(|| ServiceError::NotFound(path.to_owned()))
}

fn etag_for(content: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn map_dbus_error(unit: &str, error: zbus::Error) -> ServiceError {
    if let zbus::Error::MethodError(name, ..) = &error {
        let name = name.as_str();
        if name.contains("NoSuchUnit") || name.contains("FileNotFound") || name.contains("LoadFileFailed") {
            return ServiceError::NotFound(unit.to_owned());
        }
    }
    ServiceError::Other(error.into())
}
