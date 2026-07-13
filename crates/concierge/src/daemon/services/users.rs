use async_trait::async_trait;
use concierge_api::UserInfo;

use super::{Result, ServiceError, UserService};

/// User management. Will wrap the system user database (useradd/userdel &
/// /etc/passwd) once implemented.
pub struct SystemUserService;

#[async_trait]
impl UserService for SystemUserService {
    async fn list(&self) -> Result<Vec<UserInfo>> {
        Err(ServiceError::Unimplemented)
    }
}
