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
        handlers::list_disks,
    )
)]
struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
