use std::sync::Arc;

use super::config::Config;
use super::services::storage::BtrfsStorageService;
use super::services::system::ProcSystemService;
use super::services::tls::FoyerTlsService;
use super::services::units::SystemdUnitService;
use super::services::users::SystemUserService;
use super::services::{StorageService, SystemService, TlsService, UnitService, UserService};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub system: Arc<dyn SystemService>,
    pub users: Arc<dyn UserService>,
    pub units: Arc<dyn UnitService>,
    pub storage: Arc<dyn StorageService>,
    pub tls: Arc<dyn TlsService>,
}

impl AppState {
    pub async fn new(config: Config) -> Self {
        // Shared D-Bus connection; daemon degrades gracefully without it.
        let dbus = match zbus::Connection::system().await {
            Ok(connection) => Some(connection),
            Err(error) => {
                tracing::warn!(%error, "system D-Bus unavailable, systemd integration disabled");
                None
            }
        };

        let units: Arc<dyn UnitService> = Arc::new(SystemdUnitService::new(dbus.clone()));

        Self {
            system: Arc::new(ProcSystemService::new(dbus)),
            users: Arc::new(SystemUserService),
            tls: Arc::new(FoyerTlsService::new(
                config.tls_state_dir.clone(),
                config.haproxy_config_path.clone(),
                units.clone(),
            )),
            units,
            storage: Arc::new(BtrfsStorageService::new(
                config.data_mount_point.clone(),
                config.btrfs_bin.clone(),
            )),
            config: Arc::new(config),
        }
    }
}
