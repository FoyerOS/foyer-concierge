use anyhow::Context;
use async_trait::async_trait;
use concierge_api::{MemoryStatus, SystemStatus};

use super::units::SystemdManagerProxy;
use super::{Result, SystemService};

/// The one real service in the base: status from /proc plus the systemd
/// version over D-Bus, proving both plumbing paths end-to-end.
pub struct ProcSystemService {
    dbus: Option<zbus::Connection>,
}

impl ProcSystemService {
    pub fn new(dbus: Option<zbus::Connection>) -> Self {
        Self { dbus }
    }

    async fn systemd_version(&self) -> Option<String> {
        let connection = self.dbus.as_ref()?;
        let result = async {
            SystemdManagerProxy::new(connection).await?.version().await
        }
        .await;
        match result {
            Ok(version) => Some(version),
            Err(error) => {
                tracing::warn!(%error, "cannot query systemd version over D-Bus");
                None
            }
        }
    }
}

#[async_trait]
impl SystemService for ProcSystemService {
    async fn status(&self) -> Result<SystemStatus> {
        Ok(SystemStatus {
            hostname: read_hostname()?,
            uptime_secs: read_uptime()?,
            load_avg: read_loadavg()?,
            memory: read_meminfo()?,
            systemd_version: self.systemd_version().await,
        })
    }
}

fn read_hostname() -> anyhow::Result<String> {
    let hostname = nix::unistd::gethostname().context("gethostname failed")?;
    Ok(hostname.to_string_lossy().into_owned())
}

fn read_uptime() -> anyhow::Result<u64> {
    let raw = std::fs::read_to_string("/proc/uptime")?;
    let secs: f64 = raw
        .split_whitespace()
        .next()
        .context("empty /proc/uptime")?
        .parse()?;
    Ok(secs as u64)
}

fn read_loadavg() -> anyhow::Result<[f64; 3]> {
    let raw = std::fs::read_to_string("/proc/loadavg")?;
    let mut fields = raw.split_whitespace();
    let mut load = [0.0; 3];
    for slot in &mut load {
        *slot = fields.next().context("short /proc/loadavg")?.parse()?;
    }
    Ok(load)
}

fn read_meminfo() -> anyhow::Result<MemoryStatus> {
    let raw = std::fs::read_to_string("/proc/meminfo")?;
    let field = |name: &str| -> anyhow::Result<u64> {
        raw.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim_start_matches(':').split_whitespace().next())
            .with_context(|| format!("{name} missing from /proc/meminfo"))?
            .parse()
            .map_err(Into::into)
    };
    Ok(MemoryStatus {
        total_kib: field("MemTotal")?,
        available_kib: field("MemAvailable")?,
    })
}
