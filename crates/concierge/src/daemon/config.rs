use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Unix socket for the CLI. Trusted transport: protected by file
    /// permissions, no session auth.
    pub socket_path: PathBuf,
    /// TCP address for the WebGUI/API; `listen = false` disables TCP
    /// entirely (unix socket only).
    #[serde(deserialize_with = "deserialize_listen")]
    pub listen: Option<SocketAddr>,
    /// Group whose members may log in over TCP.
    pub admin_group: String,
    /// PAM service name used for login (file in /etc/pam.d/).
    pub pam_service: String,
    /// Where the HTTPS CA and leaf certificate/state are persisted.
    pub tls_state_dir: PathBuf,
    /// haproxy config file concierge regenerates when TLS is toggled.
    pub haproxy_config_path: PathBuf,
    /// Where the `/data` btrfs pool is mounted.
    pub data_mount_point: PathBuf,
    /// `btrfs(8)` binary storage operations shell out to.
    pub btrfs_bin: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: "/run/foyer/concierge.sock".into(),
            listen: Some(([0, 0, 0, 0], 8080).into()),
            admin_group: "foyer-admin".into(),
            pam_service: "foyer-concierge".into(),
            tls_state_dir: "/var/lib/foyer/tls".into(),
            haproxy_config_path: "/etc/haproxy/haproxy.cfg".into(),
            data_mount_point: "/data".into(),
            // The oe-core btrfs-tools recipe installs to /usr/bin, not /usr/sbin.
            btrfs_bin: "/usr/bin/btrfs".into(),
        }
    }
}

fn deserialize_listen<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SocketAddr>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Listen {
        Address(SocketAddr),
        Toggle(bool),
    }
    Ok(match Listen::deserialize(deserializer)? {
        Listen::Address(address) => Some(address),
        Listen::Toggle(false) => None,
        Listen::Toggle(true) => Config::default().listen,
    })
}

impl Config {
    /// Load from `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(raw) => {
                toml::from_str(&raw).with_context(|| format!("invalid config {}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file, using defaults");
                Ok(Self::default())
            }
            Err(error) => {
                Err(error).with_context(|| format!("cannot read config {}", path.display()))
            }
        }
    }
}
