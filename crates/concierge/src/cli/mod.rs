//! Client-side subcommands. Each one is a thin wrapper over the daemon's
//! REST API reached through the unix socket.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use concierge_api::{
    DiskInfo, HealthResponse, ServiceConfigFile, ServiceInfo, SystemStatus,
    UpdateServiceConfigRequest, UserInfo,
};

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
    /// Enable a service so it starts at boot
    Enable {
        name: String,
    },
    /// Disable a service
    Disable {
        name: String,
    },
    /// View or edit a service's mapped config file
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Print a service's config file
    Get {
        name: String,
        /// Which config file, if the service has more than one
        path: Option<String>,
    },
    /// Replace a service's config file
    Set {
        name: String,
        /// Which config file, if the service has more than one
        path: Option<String>,
        /// Read new content from this file instead of stdin
        #[arg(long)]
        file: Option<PathBuf>,
    },
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
        Command::Service(ServiceCommand::Enable { name }) => service_enable(&client, &name).await,
        Command::Service(ServiceCommand::Disable { name }) => {
            service_disable(&client, &name).await
        }
        Command::Service(ServiceCommand::Config(ConfigCommand::Get { name, path })) => {
            config_get(&client, &name, path.as_deref()).await
        }
        Command::Service(ServiceCommand::Config(ConfigCommand::Set { name, path, file })) => {
            config_set(&client, &name, path.as_deref(), file.as_deref()).await
        }
        Command::Storage(StorageCommand::Disks) => storage_disks(&client).await,
    };

    // Not-yet-implemented server features get a clean message, not a backtrace.
    if let Err(error) = &result
        && let Some(api_error) = error.downcast_ref::<ApiError>()
    {
        match api_error.body.code.as_str() {
            "unimplemented" => {
                eprintln!("this command is not implemented by the daemon yet");
                std::process::exit(1);
            }
            "conflict" => {
                eprintln!("config changed concurrently, try again");
                std::process::exit(1);
            }
            _ => {}
        }
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
            "{}\thealth={:?}\tenabled={}\tactive={}\t{}",
            service.name,
            service.health,
            service.enabled,
            service.active,
            service.description
        );
        for path in &service.config_paths {
            println!("\tconfig: {path}");
        }
    }
    Ok(())
}

async fn service_enable(client: &Client, name: &str) -> anyhow::Result<()> {
    let service: ServiceInfo = client
        .post_json(&format!("/api/v1/services/{name}/enable"))
        .await?;
    println!("{}: enabled={}", service.name, service.enabled);
    Ok(())
}

async fn service_disable(client: &Client, name: &str) -> anyhow::Result<()> {
    let service: ServiceInfo = client
        .post_json(&format!("/api/v1/services/{name}/disable"))
        .await?;
    println!("{}: enabled={}", service.name, service.enabled);
    Ok(())
}

/// Resolve which config path to act on: the caller's explicit choice, or
/// the service's only mapped config file if it has exactly one.
async fn resolve_config_path(
    client: &Client,
    name: &str,
    path: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(path) = path {
        return Ok(path.to_owned());
    }
    let services: Vec<ServiceInfo> = client.get_json("/api/v1/services").await?;
    let service = services
        .into_iter()
        .find(|service| service.name == name)
        .ok_or_else(|| anyhow::anyhow!("no such service: {name}"))?;
    match service.config_paths.as_slice() {
        [] => anyhow::bail!("{name} has no config file mapped"),
        [only] => Ok(only.clone()),
        multiple => anyhow::bail!(
            "{name} has multiple config files, pick one: {}",
            multiple.join(", ")
        ),
    }
}

async fn config_get(client: &Client, name: &str, path: Option<&str>) -> anyhow::Result<()> {
    let path = resolve_config_path(client, name, path).await?;
    let config: ServiceConfigFile = client
        .get_json(&format!("/api/v1/services/{name}/config?path={path}"))
        .await?;
    print!("{}", config.content);
    Ok(())
}

async fn config_set(
    client: &Client,
    name: &str,
    path: Option<&str>,
    file: Option<&Path>,
) -> anyhow::Result<()> {
    let path = resolve_config_path(client, name, path).await?;
    let current: ServiceConfigFile = client
        .get_json(&format!("/api/v1/services/{name}/config?path={path}"))
        .await?;

    let content = match file {
        Some(file) => std::fs::read_to_string(file)?,
        None => std::io::read_to_string(std::io::stdin())?,
    };

    let updated: ServiceConfigFile = client
        .put_json(
            &format!("/api/v1/services/{name}/config?path={path}"),
            &UpdateServiceConfigRequest {
                content,
                etag: current.etag,
            },
        )
        .await?;
    println!("wrote {}", updated.path);
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
