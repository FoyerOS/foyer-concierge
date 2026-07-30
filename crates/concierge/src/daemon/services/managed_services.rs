//! Static registry of the systemd units concierge manages. This is the
//! source of truth for two things: which units show up in `service list`
//! at all (deliberately curated, not every unit systemd has loaded), and
//! which config file(s), if any, are viewable/editable for a given unit.
//!
//! Deliberately not a generic file browser: the API only ever takes a unit
//! *name* plus a path that must match an entry here, never an arbitrary
//! caller-supplied path, so there is no path-traversal surface.

pub struct ManagedService {
    pub unit: &'static str,
    /// Config files mapped to this unit; empty if none is exposed yet.
    pub config_paths: &'static [&'static str],
}

pub const MANAGED_SERVICES: &[ManagedService] = &[
    ManagedService {
        unit: "foyer-concierge.service",
        config_paths: &["/etc/foyer/concierge.toml"],
    },
    ManagedService {
        unit: "haproxy.service",
        config_paths: &["/etc/haproxy/haproxy.cfg"],
    },
];

pub fn find(unit: &str) -> Option<&'static ManagedService> {
    MANAGED_SERVICES.iter().find(|entry| entry.unit == unit)
}
