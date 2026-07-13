use async_trait::async_trait;
use concierge_api::DiskInfo;

use super::{Result, ServiceError, StorageService};

/// Storage management. Intended backend: UDisks2 over D-Bus (`udisks2` crate).
pub struct UdisksStorageService;

#[async_trait]
impl StorageService for UdisksStorageService {
    async fn disks(&self) -> Result<Vec<DiskInfo>> {
        Err(ServiceError::Unimplemented)
    }
}
