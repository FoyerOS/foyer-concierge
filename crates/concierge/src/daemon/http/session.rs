//! Session middleware: UDS trusted by file permissions, TCP requires PAM login.

use axum::Extension;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;
use tower_sessions::Session;

use super::error::ApiError;

pub const SESSION_USERNAME_KEY: &str = "username";

/// Request transport: UDS or TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Uds,
    Tcp,
}

pub async fn require_auth(
    Extension(transport): Extension<Transport>,
    session: Session,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if transport == Transport::Tcp {
        let username: Option<String> = session
            .get(SESSION_USERNAME_KEY)
            .await
            .map_err(|error| ApiError::Internal(error.into()))?;
        if username.is_none() {
            return Err(ApiError::Unauthorized("authentication required"));
        }
    }
    Ok(next.run(request).await)
}
