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
    Unimplemented,
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
            Self::Unimplemented => (
                StatusCode::NOT_IMPLEMENTED,
                "unimplemented",
                "not implemented yet".to_owned(),
            ),
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
            ServiceError::Other(error) => Self::Internal(error),
        }
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::InvalidCredentials => Self::Unauthorized("invalid username or password"),
            AuthError::NotAuthorized => Self::Forbidden,
            AuthError::Internal(error) => Self::Internal(error),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error)
    }
}
