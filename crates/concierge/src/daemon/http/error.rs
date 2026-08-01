use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use concierge_api::ApiErrorBody;

use crate::daemon::auth::AuthError;
use crate::daemon::services::ServiceError;

#[derive(Debug)]
pub enum ApiError {
    Unauthorized(&'static str),
    Forbidden,
    PasswordChangeRequired,
    Unimplemented,
    NotFound(String),
    Conflict(String),
    Internal(anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized(message) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", message.to_owned())
            }
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "not allowed".to_owned(),
            ),
            Self::PasswordChangeRequired => (
                StatusCode::FORBIDDEN,
                "password_change_required",
                "password change required before login".to_owned(),
            ),
            Self::Unimplemented => (
                StatusCode::NOT_IMPLEMENTED,
                "unimplemented",
                "not implemented yet".to_owned(),
            ),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Internal(error) => {
                tracing::error!(error = %format!("{error:#}"), "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".to_owned(),
                )
            }
        };
        let body = ApiErrorBody {
            code: code.to_owned(),
            message,
        };
        (status, Json(body)).into_response()
    }
}

impl From<ServiceError> for ApiError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Unimplemented => Self::Unimplemented,
            ServiceError::NotFound(message) => Self::NotFound(message),
            ServiceError::Conflict(message) => Self::Conflict(message),
            ServiceError::Other(error) => Self::Internal(error),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::InvalidCredentials => Self::Unauthorized("invalid username or password"),
            AuthError::NotAuthorized => Self::Forbidden,
            AuthError::PasswordChangeRequired => Self::PasswordChangeRequired,
            AuthError::Internal(error) => Self::Internal(error),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}
