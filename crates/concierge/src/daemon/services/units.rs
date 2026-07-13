use async_trait::async_trait;
use concierge_api::ServiceInfo;

use super::{Result, ServiceError, UnitService};

/// Proxy to the systemd manager on the system bus. Only what the base needs;
/// unit control methods (StartUnit, EnableUnitFiles, ...) get added here as
/// the units feature is implemented.
#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait SystemdManager {
    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;
}

/// Unit management backed by systemd over D-Bus.
pub struct SystemdUnitService {
    #[expect(dead_code, reason = "plumbing for the real implementation")]
    connection: Option<zbus::Connection>,
}

impl SystemdUnitService {
    pub fn new(connection: Option<zbus::Connection>) -> Self {
        Self { connection }
    }
}

#[async_trait]
impl UnitService for SystemdUnitService {
    async fn list(&self) -> Result<Vec<ServiceInfo>> {
        Err(ServiceError::Unimplemented)
    }
}
