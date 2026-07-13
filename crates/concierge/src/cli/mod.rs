//! Client-side subcommands. Each one is a thin wrapper over the daemon's
//! REST API reached through the unix socket.

use std::path::Path;

use clap::Subcommand;
use concierge_api::{DiskInfo, HealthResponse, ServiceInfo, SystemStatus, UserInfo};

use crate::Command;
use crate::client::{ApiError, Client};

#[derive(Subcommand)]
pub enum UserCommand {
    /// List system users
    List,
}

#[derive(Subcommand)]
pub enum ServiceCommand {
    /// List managed services
    List,
}

#[derive(Subcommand)]
pub enum StorageCommand {
    /// List disks
    Disks,
}

pub async fn run(socket: &Path, command: Command) -> anyhow::Result<()> {
    let client = Client::new(socket);
    let result = match command {
        Command::Daemon { .. } => unreachable!("handled in main"),
        Command::Health => health(&client).await,
        Command::Status => status(&client).await,
        Command::User(UserCommand::List) => user_list(&client).await,
        Command::Service(ServiceCommand::List) => service_list(&client).await,
        Command::Storage(StorageCommand::Disks) => storage_disks(&client).await,
    };

    // Not-yet-implemented server features get a clean message, not a backtrace.
    if let Err(error) = &result
        && let Some(api_error) = error.downcast_ref::<ApiError>()
        && api_error.body.code == "unimplemented"
    {
        eprintln!("this command is not implemented by the daemon yet");
        std::process::exit(1);
    }
    result
}

async fn health(client: &Client) -> anyhow::Result<()> {
    let health: HealthResponse = client.get_json("/api/v1/health").await?;
    println!("daemon is {:?} (version {})", health.status, health.version);
    Ok(())
}

async fn status(client: &Client) -> anyhow::Result<()> {
    let status: SystemStatus = client.get_json("/api/v1/system/status").await?;
    println!("hostname : {}", status.hostname);
    println!("uptime   : {}", format_duration(status.uptime_secs));
    println!(
        "load     : {:.2} {:.2} {:.2}",
        status.load_avg[0], status.load_avg[1], status.load_avg[2]
    );
    println!(
        "memory   : {} MiB available / {} MiB total",
        status.memory.available_kib / 1024,
        status.memory.total_kib / 1024
    );
    match status.systemd_version {
        Some(version) => println!("systemd  : {version}"),
        None => println!("systemd  : unreachable over D-Bus"),
    }
    Ok(())
}

async fn user_list(client: &Client) -> anyhow::Result<()> {
    let users: Vec<UserInfo> = client.get_json("/api/v1/users").await?;
    for user in users {
        println!("{}\t{}", user.uid, user.name);
    }
    Ok(())
}

async fn service_list(client: &Client) -> anyhow::Result<()> {
    let services: Vec<ServiceInfo> = client.get_json("/api/v1/services").await?;
    for service in services {
        println!(
            "{}\tenabled={}\tactive={}",
            service.name, service.enabled, service.active
        );
    }
    Ok(())
}

async fn storage_disks(client: &Client) -> anyhow::Result<()> {
    let disks: Vec<DiskInfo> = client.get_json("/api/v1/storage/disks").await?;
    for disk in disks {
        println!(
            "{}\t{} GiB\t{}",
            disk.device,
            disk.size_bytes / (1024 * 1024 * 1024),
            disk.model.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

fn format_duration(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else {
        format!("{hours}h {minutes}m")
    }
}
