pub mod auth;
pub mod config;
pub mod http;
pub mod services;
pub mod state;

use std::path::Path;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt, registry};

use self::config::Config;
use self::state::AppState;

pub async fn run(config_path: &Path) -> anyhow::Result<()> {
    init_tracing();
    let config = Config::load(config_path)?;
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        socket = %config.socket_path.display(),
        listen = ?config.listen,
        "starting foyer-concierge"
    );
    let state = AppState::new(config).await;
    http::serve(state).await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Under systemd, log straight to the journal (structured fields
    // preserved); otherwise plain stderr for interactive runs.
    if std::env::var_os("JOURNAL_STREAM").is_some()
        && let Ok(journald) = tracing_journald::layer()
    {
        registry().with(filter).with(journald).init();
        return;
    }
    registry().with(filter).with(fmt::layer()).init();
}
