mod error;
mod handlers;
mod openapi;
mod session;
mod webgui;

use std::os::unix::fs::PermissionsExt;

use anyhow::Context;
use axum::routing::{get, post};
use axum::{Extension, Router, middleware};
use tower_http::trace::TraceLayer;
use tower_sessions::{MemoryStore, SessionManagerLayer};

use self::session::Transport;
use super::state::AppState;

fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(handlers::health))
        .route("/auth/login", post(handlers::login))
        .route("/auth/change-password", post(handlers::change_password));

    let protected = Router::new()
        .route("/auth/logout", post(handlers::logout))
        .route("/auth/session", get(handlers::session_info))
        .route("/system/status", get(handlers::system_status))
        .route("/users", get(handlers::list_users))
        .route("/services", get(handlers::list_services))
        .route("/services/{name}/enable", post(handlers::enable_service))
        .route("/services/{name}/disable", post(handlers::disable_service))
        .route(
            "/services/{name}/config",
            get(handlers::get_service_config).put(handlers::update_service_config),
        )
        .route("/storage/disks", get(handlers::list_disks))
        .route("/storage/pool", get(handlers::pool_status))
        .route("/storage/pool/devices", post(handlers::pool_add))
        .route("/storage/pool/devices/remove", post(handlers::pool_remove))
        .route("/storage/pool/grow", post(handlers::pool_grow))
        .route("/tls/status", get(handlers::tls_status))
        .route("/tls/enable", post(handlers::tls_enable))
        .route("/tls/disable", post(handlers::tls_disable))
        .route("/tls/ca", get(handlers::tls_ca).put(handlers::tls_set_ca))
        .route_layer(middleware::from_fn(session::require_auth));

    // In-memory sessions; no TLS so secure=false.
    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_name("concierge_session")
        .with_secure(false);

    Router::new()
        .nest(&format!("/api/{}", concierge_api::API_VERSION), public.merge(protected))
        .route("/api/openapi.json", get(openapi::openapi_json))
        .fallback(webgui::spa_fallback)
        .layer(session_layer)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Bind sockets, notify systemd, serve until shutdown.
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let config = state.config.clone();
    let app = router(state);

    let socket_path = &config.socket_path;
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    match std::fs::remove_file(socket_path) {
        Ok(()) => tracing::debug!(path = %socket_path.display(), "removed stale socket"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("cannot remove stale socket"),
    }
    let uds_listener = tokio::net::UnixListener::bind(socket_path)
        .with_context(|| format!("cannot bind {}", socket_path.display()))?;
    // Clean up socket on all exit paths.
    struct SocketGuard<'a>(&'a std::path::Path);
    impl Drop for SocketGuard<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0);
        }
    }
    let _socket_guard = SocketGuard(socket_path);
    // Socket permissions authenticate CLI callers.
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))
        .context("cannot set socket permissions")?;
    tracing::info!(path = %socket_path.display(), "listening on unix socket");

    let uds_app = app.clone().layer(Extension(Transport::Uds));
    let uds_serve =
        axum::serve(uds_listener, uds_app).with_graceful_shutdown(shutdown_signal());

    let tcp_serve = match config.listen {
        Some(address) => {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .with_context(|| format!("cannot bind {address}"))?;
            tracing::info!(%address, "listening on tcp");
            let tcp_app = app.layer(Extension(Transport::Tcp));
            Some(axum::serve(listener, tcp_app).with_graceful_shutdown(shutdown_signal()))
        }
        None => {
            tracing::info!("tcp listener disabled by config");
            None
        }
    };

    let _ = sd_notify::notify(&[sd_notify::NotifyState::Ready]);

    match tcp_serve {
        Some(tcp_serve) => tokio::try_join!(uds_serve, tcp_serve).map(|_| ()),
        None => uds_serve.await,
    }
    .context("server error")
}

async fn shutdown_signal() {
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("cannot install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
    tracing::info!("shutdown requested");
}
