use axum::Json;
use axum::extract::State;
use concierge_api::{
    ApiErrorBody, DiskInfo, HealthResponse, HealthStatus, LoginRequest, ServiceInfo, SessionInfo,
    SystemStatus, UserInfo,
};
use tower_sessions::Session;

use super::error::ApiError;
use super::session::SESSION_USERNAME_KEY;
use crate::daemon::auth;
use crate::daemon::state::AppState;

type ApiResult<T> = Result<Json<T>, ApiError>;

#[utoipa::path(get, path = "/api/v1/health", responses((status = 200, body = HealthResponse)))]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: HealthStatus::Ok,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

#[utoipa::path(post, path = "/api/v1/auth/login", request_body = LoginRequest, responses(
    (status = 200, body = SessionInfo),
    (status = 401, body = ApiErrorBody),
    (status = 403, body = ApiErrorBody),
))]
pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<LoginRequest>,
) -> ApiResult<SessionInfo> {
    auth::login(&state.config, &request.username, &request.password).await?;
    // Cycle session ID on privilege change.
    session
        .cycle_id()
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    session
        .insert(SESSION_USERNAME_KEY, request.username.clone())
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    Ok(Json(SessionInfo {
        username: request.username,
    }))
}

#[utoipa::path(post, path = "/api/v1/auth/logout", responses((status = 204)))]
pub async fn logout(session: Session) -> Result<axum::http::StatusCode, ApiError> {
    session
        .flush()
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v1/auth/session", responses(
    (status = 200, body = SessionInfo),
    (status = 401, body = ApiErrorBody),
))]
pub async fn session_info(session: Session) -> ApiResult<SessionInfo> {
    let username: Option<String> = session
        .get(SESSION_USERNAME_KEY)
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    match username {
        Some(username) => Ok(Json(SessionInfo { username })),
        // UDS authenticated by transport.
        None => Ok(Json(SessionInfo {
            username: "root".to_owned(),
        })),
    }
}

#[utoipa::path(get, path = "/api/v1/system/status", responses((status = 200, body = SystemStatus)))]
pub async fn system_status(State(state): State<AppState>) -> ApiResult<SystemStatus> {
    Ok(Json(state.system.status().await?))
}

#[utoipa::path(get, path = "/api/v1/users", responses(
    (status = 200, body = [UserInfo]),
    (status = 501, body = ApiErrorBody),
))]
pub async fn list_users(State(state): State<AppState>) -> ApiResult<Vec<UserInfo>> {
    Ok(Json(state.users.list().await?))
}

#[utoipa::path(get, path = "/api/v1/services", responses(
    (status = 200, body = [ServiceInfo]),
    (status = 501, body = ApiErrorBody),
))]
pub async fn list_services(State(state): State<AppState>) -> ApiResult<Vec<ServiceInfo>> {
    Ok(Json(state.units.list().await?))
}

#[utoipa::path(get, path = "/api/v1/storage/disks", responses(
    (status = 200, body = [DiskInfo]),
    (status = 501, body = ApiErrorBody),
))]
pub async fn list_disks(State(state): State<AppState>) -> ApiResult<Vec<DiskInfo>> {
    Ok(Json(state.storage.disks().await?))
}
