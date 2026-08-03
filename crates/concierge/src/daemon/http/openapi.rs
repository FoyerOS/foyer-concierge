use axum::Json;
use utoipa::OpenApi;

use super::handlers;

#[derive(OpenApi)]
#[openapi(
    info(title = "foyer-concierge", description = "Foyer OS management API"),
    paths(
        handlers::health,
        handlers::login,
        handlers::logout,
        handlers::session_info,
        handlers::system_status,
        handlers::list_users,
        handlers::list_services,
        handlers::enable_service,
        handlers::disable_service,
        handlers::get_service_config,
        handlers::update_service_config,
        handlers::list_disks,
        handlers::pool_status,
        handlers::pool_add,
        handlers::pool_remove,
        handlers::pool_grow,
        handlers::tls_status,
        handlers::tls_enable,
        handlers::tls_disable,
        handlers::tls_ca,
        handlers::tls_set_ca,
    )
)]
struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
