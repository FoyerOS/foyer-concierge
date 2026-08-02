mod cli;
mod client;
mod daemon;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "concierge", version, about = "Foyer OS management daemon and CLI")]
struct Cli {
    /// Unix socket of the running daemon (client subcommands).
    #[arg(
        long,
        global = true,
        env = "CONCIERGE_SOCKET",
        default_value = "/run/foyer/concierge.sock"
    )]
    socket: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the management daemon
    Daemon {
        /// Path to the daemon configuration file
        #[arg(long, env = "CONCIERGE_CONFIG", default_value = "/etc/foyer/concierge.toml")]
        config: PathBuf,
    },
    /// Check that the daemon is alive
    Health,
    /// Show system status
    Status,
    /// Manage system users
    #[command(subcommand)]
    User(cli::UserCommand),
    /// Manage services (systemd/podman daemons)
    #[command(subcommand)]
    Service(cli::ServiceCommand),
    /// Manage storage
    #[command(subcommand)]
    Storage(cli::StorageCommand),
    /// Manage HTTPS termination at haproxy
    #[command(subcommand)]
    Tls(cli::TlsCommand),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    match args.command {
        Command::Daemon { config } => daemon::run(&config).await,
        command => cli::run(&args.socket, command).await,
    }
}
