//! Static registry of managed services haproxy can route to by subdomain.
//!
//! Curated on purpose, same reasoning as `managed_services`: only apps with
//! their own web UI belong here (not postgres/redis, which speak no HTTP).
//! `TlsService` (see `tls.rs`) consults this to build one `acl`/`use_backend`
//! pair per currently-enabled entry whenever HTTPS mode is on; wiring in a
//! new app means adding both a `ManagedService` and a `RoutableService`.

pub struct RoutableService {
    /// Must match a `ManagedService::unit` so `UnitService::list` can report
    /// whether it's currently enabled.
    pub unit: &'static str,
    /// Subdomain label; the full hostname routed is "<label>.<domain>".
    pub label: &'static str,
    /// Where haproxy forwards matching requests. Both apps below publish
    /// their port to the host (see their .container units), so haproxy
    /// (which runs on the host, not in a container) reaches them over
    /// loopback regardless of which podman network they're otherwise on.
    pub backend_addr: &'static str,
}

pub const ROUTABLE_SERVICES: &[RoutableService] = &[
    RoutableService {
        unit: "homeassistant.service",
        label: "homeassistant",
        backend_addr: "127.0.0.1:8123",
    },
    RoutableService {
        unit: "affine.service",
        label: "affine",
        backend_addr: "127.0.0.1:3010",
    },
];

pub fn is_routable(unit: &str) -> bool {
    ROUTABLE_SERVICES.iter().any(|route| route.unit == unit)
}
