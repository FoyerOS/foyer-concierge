use std::sync::Arc;

use super::config::Config;
use super::services::storage::UdisksStorageService;
use super::services::system::ProcSystemService;
use super::services::units::SystemdUnitService;
use super::services::users::SystemUserService;
use super::services::{StorageService, SystemService, UnitService, UserService};

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub system: Arc<dyn SystemService>,
    pub users: Arc<dyn UserService>,
    pub units: Arc<dyn UnitService>,
    pub storage: Arc<dyn StorageService>,
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

        Self {
            config: Arc::new(config),
            system: Arc::new(ProcSystemService::new(dbus.clone())),
            users: Arc::new(SystemUserService),
            units: Arc::new(SystemdUnitService::new(dbus)),
            storage: Arc::new(UdisksStorageService),
        }
    }
}
