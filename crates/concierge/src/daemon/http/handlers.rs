use axum::Json;
use axum::extract::{Path, Query, State};
use concierge_api::{
    ApiErrorBody, ChangePasswordRequest, DiskInfo, EnableTlsRequest, HealthResponse, HealthStatus,
    LoginRequest, ServiceConfigFile, ServiceInfo, SessionInfo, SetCaRequest, SystemStatus,
    TlsStatus, UpdateServiceConfigRequest, UserInfo,
};
use serde::Deserialize;
use tower_sessions::Session;

use super::error::ApiError;
use super::session::SESSION_USERNAME_KEY;
use crate::daemon::auth;
use crate::daemon::services::routes;
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
    Ok(Json(establish_session(&session, request.username).await?))
}

#[utoipa::path(post, path = "/api/v1/auth/change-password", request_body = ChangePasswordRequest, responses(
    (status = 200, body = SessionInfo),
    (status = 400, body = ApiErrorBody),
    (status = 401, body = ApiErrorBody),
    (status = 403, body = ApiErrorBody),
))]
pub async fn change_password(
    State(state): State<AppState>,
    session: Session,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<SessionInfo> {
    auth::change_expired_password(
        &state.config,
        &request.username,
        &request.current_password,
        &request.new_password,
    )
    .await?;
    // Re-validate with the new password (also re-checks admin-group
    // membership) rather than trusting chauthtok succeeding implies login
    // would too.
    auth::login(&state.config, &request.username, &request.new_password).await?;
    Ok(Json(establish_session(&session, request.username).await?))
}

/// Cycle the session ID (mitigates session fixation on a privilege change)
/// and record the now-authenticated username.
async fn establish_session(session: &Session, username: String) -> Result<SessionInfo, ApiError> {
    session
        .cycle_id()
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    session
        .insert(SESSION_USERNAME_KEY, username.clone())
        .await
        .map_err(|error| ApiError::Internal(error.into()))?;
    Ok(SessionInfo { username })
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

#[utoipa::path(post, path = "/api/v1/services/{name}/enable", params(
    ("name" = String, Path, description = "systemd unit name"),
), responses(
    (status = 200, body = ServiceInfo),
    (status = 404, body = ApiErrorBody),
))]
pub async fn enable_service(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<ServiceInfo> {
    let info = state.units.enable(&name).await?;
    if routes::is_routable(&name) {
        state.tls.sync_routes().await?;
    }
    Ok(Json(info))
}

#[utoipa::path(post, path = "/api/v1/services/{name}/disable", params(
    ("name" = String, Path, description = "systemd unit name"),
), responses(
    (status = 200, body = ServiceInfo),
    (status = 404, body = ApiErrorBody),
))]
pub async fn disable_service(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<ServiceInfo> {
    let info = state.units.disable(&name).await?;
    if routes::is_routable(&name) {
        state.tls.sync_routes().await?;
    }
    Ok(Json(info))
}

#[derive(Debug, Deserialize)]
pub struct ConfigPathQuery {
    pub path: String,
}

#[utoipa::path(get, path = "/api/v1/services/{name}/config", params(
    ("name" = String, Path, description = "systemd unit name"),
    ("path" = String, Query, description = "config file path, from ServiceInfo.config_paths"),
), responses(
    (status = 200, body = ServiceConfigFile),
    (status = 404, body = ApiErrorBody),
))]
pub async fn get_service_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ConfigPathQuery>,
) -> ApiResult<ServiceConfigFile> {
    Ok(Json(state.units.get_config(&name, &query.path).await?))
}

#[utoipa::path(put, path = "/api/v1/services/{name}/config", params(
    ("name" = String, Path, description = "systemd unit name"),
    ("path" = String, Query, description = "config file path, from ServiceInfo.config_paths"),
), request_body = UpdateServiceConfigRequest, responses(
    (status = 200, body = ServiceConfigFile),
    (status = 404, body = ApiErrorBody),
    (status = 409, body = ApiErrorBody),
))]
pub async fn update_service_config(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<ConfigPathQuery>,
    Json(request): Json<UpdateServiceConfigRequest>,
) -> ApiResult<ServiceConfigFile> {
    Ok(Json(
        state
            .units
            .set_config(&name, &query.path, request.content, &request.etag)
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/v1/storage/disks", responses(
    (status = 200, body = [DiskInfo]),
    (status = 501, body = ApiErrorBody),
))]
pub async fn list_disks(State(state): State<AppState>) -> ApiResult<Vec<DiskInfo>> {
    Ok(Json(state.storage.disks().await?))
}

#[utoipa::path(get, path = "/api/v1/tls/status", responses((status = 200, body = TlsStatus)))]
pub async fn tls_status(State(state): State<AppState>) -> ApiResult<TlsStatus> {
    Ok(Json(state.tls.status().await?))
}

#[utoipa::path(post, path = "/api/v1/tls/enable", request_body = EnableTlsRequest, responses(
    (status = 200, body = TlsStatus),
))]
pub async fn tls_enable(
    State(state): State<AppState>,
    Json(request): Json<EnableTlsRequest>,
) -> ApiResult<TlsStatus> {
    Ok(Json(state.tls.enable(&request.domain).await?))
}

#[utoipa::path(post, path = "/api/v1/tls/disable", responses((status = 200, body = TlsStatus)))]
pub async fn tls_disable(State(state): State<AppState>) -> ApiResult<TlsStatus> {
    Ok(Json(state.tls.disable().await?))
}

#[utoipa::path(get, path = "/api/v1/tls/ca", responses(
    (status = 200, body = String),
    (status = 404, body = ApiErrorBody),
))]
pub async fn tls_ca(State(state): State<AppState>) -> Result<String, ApiError> {
    Ok(state.tls.ca_cert().await?)
}

#[utoipa::path(put, path = "/api/v1/tls/ca", request_body = SetCaRequest, responses(
    (status = 200, body = TlsStatus),
))]
pub async fn tls_set_ca(
    State(state): State<AppState>,
    Json(request): Json<SetCaRequest>,
) -> ApiResult<TlsStatus> {
    Ok(Json(state.tls.set_ca(request.ca_cert_pem, request.ca_key_pem).await?))
}
