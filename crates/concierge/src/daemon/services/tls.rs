//! HTTPS termination and per-service routing at haproxy.
//!
//! Concierge owns a self-signed root CA and issues a wildcard leaf
//! certificate for whatever domain the operator gives it — no public DNS or
//! a reachable port 80 required, so the same flow works on a home LAN or a
//! cloud box. Users install/trust the CA once; `set_ca` is the exit hatch
//! for power users who'd rather Foyer's cert chain to a CA they already run
//! and trust elsewhere.
//!
//! Once TLS is on, haproxy also routes each `routes::ROUTABLE_SERVICES`
//! entry that's currently enabled to its own `<label>.<domain>` subdomain
//! (`sync_routes`, called from the service enable/disable HTTP handlers) —
//! plain-HTTP mode never does this, since there's no domain to build
//! subdomains from until TLS has been turned on at least once.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use concierge_api::TlsStatus;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use super::routes::{ROUTABLE_SERVICES, RoutableService};
use super::{Result, ServiceError, TlsService, UnitService};

const CA_VALIDITY_DAYS: i64 = 3650;
const LEAF_VALIDITY_DAYS: i64 = 730;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CaSource {
    #[default]
    Generated,
    Imported,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    enabled: bool,
    domain: Option<String>,
    ca_source: CaSource,
    ca_not_after: Option<String>,
    cert_not_after: Option<String>,
}

/// Self-signed-CA-backed `TlsService`, storing everything under
/// `state_dir` and rewriting/reloading `haproxy_config_path` on every
/// enable/disable/set_ca call.
pub struct FoyerTlsService {
    state_dir: PathBuf,
    haproxy_config_path: PathBuf,
    units: Arc<dyn UnitService>,
}

impl FoyerTlsService {
    pub fn new(state_dir: PathBuf, haproxy_config_path: PathBuf, units: Arc<dyn UnitService>) -> Self {
        Self { state_dir, haproxy_config_path, units }
    }

    fn ca_cert_path(&self) -> PathBuf {
        self.state_dir.join("ca/ca-cert.pem")
    }

    fn ca_key_path(&self) -> PathBuf {
        self.state_dir.join("ca/ca-key.pem")
    }

    fn leaf_path(&self) -> PathBuf {
        self.state_dir.join("leaf/haproxy.pem")
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("state.toml")
    }

    fn load_state(&self) -> Result<State> {
        match std::fs::read_to_string(self.state_path()) {
            Ok(raw) => toml::from_str(&raw).map_err(|error| ServiceError::Other(error.into())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => Err(io_err(error)),
        }
    }

    fn save_state(&self, state: &State) -> Result<()> {
        let content = toml::to_string_pretty(state).map_err(|error| ServiceError::Other(error.into()))?;
        write_atomic(&self.state_path(), content.as_bytes(), 0o600)
    }

    fn status_from_state(&self, state: &State) -> TlsStatus {
        TlsStatus {
            enabled: state.enabled,
            domain: state.domain.clone(),
            ca_managed: state.ca_source == CaSource::Generated,
            ca_not_after: state.ca_not_after.clone(),
            cert_not_after: state.cert_not_after.clone(),
        }
    }

    /// Load the CA from disk, generating one (and recording it in `state`)
    /// if this is the first time TLS has ever been enabled.
    fn load_or_create_ca(&self, state: &mut State) -> Result<(String, String)> {
        match (
            std::fs::read_to_string(self.ca_cert_path()),
            std::fs::read_to_string(self.ca_key_path()),
        ) {
            (Ok(cert_pem), Ok(key_pem)) => Ok((cert_pem, key_pem)),
            (Err(error), _) | (_, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let ca = generate_ca()?;
                write_atomic(&self.ca_cert_path(), ca.cert_pem.as_bytes(), 0o644)?;
                write_atomic(&self.ca_key_path(), ca.key_pem.as_bytes(), 0o600)?;
                state.ca_source = CaSource::Generated;
                state.ca_not_after = Some(ca.not_after);
                Ok((ca.cert_pem, ca.key_pem))
            }
            (Err(error), _) | (_, Err(error)) => Err(io_err(error)),
        }
    }

    /// Which `ROUTABLE_SERVICES` entries are currently enabled (boot
    /// autostart, the same flag `concierge service enable/disable` flips) —
    /// haproxy routes their subdomain in TLS mode, and only in TLS mode:
    /// plain-HTTP mode has no recorded domain to build subdomains from.
    async fn active_routes(&self) -> Result<Vec<&'static RoutableService>> {
        let services = self.units.list().await?;
        Ok(ROUTABLE_SERVICES
            .iter()
            .filter(|route| services.iter().any(|info| info.name == route.unit && info.enabled))
            .collect())
    }

    /// Single funnel for every state change that affects haproxy.cfg: TLS
    /// on/off, a new/imported CA re-issuing the leaf, or a routable
    /// service's enabled flag flipping. Always re-renders from `state` plus
    /// the live routable-service list, so it can never drift from either.
    async fn regenerate_haproxy(&self, state: &State) -> Result<()> {
        let routes = if state.enabled { self.active_routes().await? } else { Vec::new() };
        let leaf_path = state.enabled.then(|| self.leaf_path());
        self.write_haproxy_cfg(state.domain.as_deref(), leaf_path.as_deref(), &routes)?;
        self.units.reload("haproxy.service").await?;
        Ok(())
    }

    /// Render `haproxy_config_path`, validate it with `haproxy -c`, and
    /// only then atomically replace the live file — a bad render can never
    /// land as the on-disk config.
    fn write_haproxy_cfg(
        &self,
        domain: Option<&str>,
        leaf_path: Option<&Path>,
        routes: &[&'static RoutableService],
    ) -> Result<()> {
        let rendered = match (leaf_path, domain) {
            (Some(leaf_path), Some(domain)) => render_tls_cfg(leaf_path, domain, routes),
            // Defensive fallback: TLS state without a recorded domain
            // shouldn't happen (enable() always sets both together), but
            // plain mode is always a safe, valid config to fall back to.
            _ => render_plain_cfg(),
        };

        let parent = self.haproxy_config_path.parent().unwrap_or_else(|| Path::new("/"));
        std::fs::create_dir_all(parent).map_err(io_err)?;
        let temp_path = parent.join(".haproxy.cfg.tmp");
        std::fs::write(&temp_path, &rendered).map_err(io_err)?;

        let validation = std::process::Command::new("/usr/sbin/haproxy")
            .args(["-c", "-q", "-f"])
            .arg(&temp_path)
            .output();
        match validation {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(ServiceError::Other(anyhow::anyhow!(
                    "generated haproxy config failed validation: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(io_err(error));
            }
        }

        std::fs::rename(&temp_path, &self.haproxy_config_path).map_err(io_err)
    }

    /// Issue a fresh leaf cert for `domain` signed by the given CA, install
    /// it, and record its expiry in `state`. Does not touch haproxy.cfg or
    /// reload — callers do that once, after any CA work is also done.
    fn issue_and_install_leaf(
        &self,
        domain: &str,
        ca_cert_pem: &str,
        ca_key_pem: &str,
        state: &mut State,
    ) -> Result<()> {
        let leaf = issue_leaf(domain, ca_cert_pem, ca_key_pem)?;
        write_atomic(&self.leaf_path(), leaf.combined_pem.as_bytes(), 0o600)?;
        state.domain = Some(domain.to_owned());
        state.cert_not_after = Some(leaf.not_after);
        Ok(())
    }
}

#[async_trait]
impl TlsService for FoyerTlsService {
    async fn status(&self) -> Result<TlsStatus> {
        Ok(self.status_from_state(&self.load_state()?))
    }

    async fn enable(&self, domain: &str) -> Result<TlsStatus> {
        let mut state = self.load_state()?;
        let (ca_cert_pem, ca_key_pem) = self.load_or_create_ca(&mut state)?;
        self.issue_and_install_leaf(domain, &ca_cert_pem, &ca_key_pem, &mut state)?;
        state.enabled = true;

        self.regenerate_haproxy(&state).await?;
        self.save_state(&state)?;
        Ok(self.status_from_state(&state))
    }

    async fn disable(&self) -> Result<TlsStatus> {
        let mut state = self.load_state()?;
        state.enabled = false;

        self.regenerate_haproxy(&state).await?;
        self.save_state(&state)?;
        Ok(self.status_from_state(&state))
    }

    async fn ca_cert(&self) -> Result<String> {
        std::fs::read_to_string(self.ca_cert_path()).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ServiceError::NotFound(
                    "no CA yet; run `concierge tls enable` or `concierge tls set-ca` first".into(),
                )
            } else {
                io_err(error)
            }
        })
    }

    async fn set_ca(&self, ca_cert_pem: String, ca_key_pem: String) -> Result<TlsStatus> {
        let not_after = validate_ca_pair(&ca_cert_pem, &ca_key_pem)?;

        write_atomic(&self.ca_cert_path(), ca_cert_pem.as_bytes(), 0o644)?;
        write_atomic(&self.ca_key_path(), ca_key_pem.as_bytes(), 0o600)?;

        let mut state = self.load_state()?;
        state.ca_source = CaSource::Imported;
        state.ca_not_after = Some(not_after);

        if state.enabled {
            let domain = state.domain.clone().ok_or_else(|| {
                ServiceError::Other(anyhow::anyhow!("tls is enabled but has no recorded domain"))
            })?;
            self.issue_and_install_leaf(&domain, &ca_cert_pem, &ca_key_pem, &mut state)?;
            self.regenerate_haproxy(&state).await?;
        }

        self.save_state(&state)?;
        Ok(self.status_from_state(&state))
    }

    async fn sync_routes(&self) -> Result<()> {
        let state = self.load_state()?;
        if state.enabled {
            self.regenerate_haproxy(&state).await?;
        }
        Ok(())
    }
}

struct GeneratedCa {
    cert_pem: String,
    key_pem: String,
    not_after: String,
}

fn generate_ca() -> Result<GeneratedCa> {
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(cert_err)?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.distinguished_name.push(DnType::CommonName, "Foyer OS Local CA");
    params.distinguished_name.push(DnType::OrganizationName, "Foyer OS");
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);

    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(CA_VALIDITY_DAYS);
    let not_after = format_rfc3339(params.not_after)?;

    let key_pair = KeyPair::generate().map_err(cert_err)?;
    let cert = params.self_signed(&key_pair).map_err(cert_err)?;

    Ok(GeneratedCa { cert_pem: cert.pem(), key_pem: key_pair.serialize_pem(), not_after })
}

struct GeneratedLeaf {
    /// Leaf certificate followed by its private key, PEM-concatenated —
    /// the shape haproxy's `crt` directive expects in a single file.
    combined_pem: String,
    not_after: String,
}

fn issue_leaf(domain: &str, ca_cert_pem: &str, ca_key_pem: &str) -> Result<GeneratedLeaf> {
    let ca_key_pair = KeyPair::from_pem(ca_key_pem).map_err(cert_err)?;
    let issuer = Issuer::from_ca_cert_pem(ca_cert_pem, ca_key_pair).map_err(cert_err)?;

    // Wildcard SAN so newly routable services (see routes.rs) get a valid
    // subdomain immediately, without reissuing the leaf on every toggle.
    let sans = vec![domain.to_owned(), format!("*.{domain}")];
    let mut params = CertificateParams::new(sans).map_err(cert_err)?;
    params.distinguished_name.push(DnType::CommonName, domain);
    params.use_authority_key_identifier_extension = true;
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.extended_key_usages.push(ExtendedKeyUsagePurpose::ServerAuth);

    let now = OffsetDateTime::now_utc();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(LEAF_VALIDITY_DAYS);
    let not_after = format_rfc3339(params.not_after)?;

    let leaf_key = KeyPair::generate().map_err(cert_err)?;
    let leaf_cert = params.signed_by(&leaf_key, &issuer).map_err(cert_err)?;

    let combined_pem = format!("{}{}", leaf_cert.pem(), leaf_key.serialize_pem());
    Ok(GeneratedLeaf { combined_pem, not_after })
}

/// Confirm `key_pem` is actually the private half of `cert_pem`'s public
/// key before accepting an imported CA — a mismatched pair would silently
/// produce leaf certificates nothing can validate.
fn validate_ca_pair(cert_pem: &str, key_pem: &str) -> Result<String> {
    let key_pair = KeyPair::from_pem(key_pem).map_err(cert_err)?;

    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|error| ServiceError::Other(anyhow::anyhow!("invalid CA certificate: {error}")))?;
    let cert = pem
        .parse_x509()
        .map_err(|error| ServiceError::Other(anyhow::anyhow!("invalid CA certificate: {error}")))?;

    let cert_public_key: &[u8] = cert.tbs_certificate.subject_pki.subject_public_key.as_ref();
    if cert_public_key != key_pair.public_key_raw() {
        return Err(ServiceError::Other(anyhow::anyhow!(
            "the supplied CA key does not match the supplied CA certificate"
        )));
    }

    format_rfc3339(cert.tbs_certificate.validity().not_after.to_datetime())
}

fn render_plain_cfg() -> String {
    HAPROXY_HEADER.to_owned()
        + "# Concierge owns and regenerates this file; `concierge tls enable --domain\n\
           # ...` switches this frontend to terminate HTTPS on :443 instead of plain\n\
           # HTTP on :80. Manual edits survive only until the next `concierge tls\n\
           # enable`/`disable` call.\n\
           frontend concierge_in\n\
           \x20   bind *:80\n\
           \x20   default_backend concierge\n\
           \n\
           backend concierge\n\
           \x20   server concierge1 127.0.0.1:8080 check\n"
}

fn render_tls_cfg(leaf_path: &Path, domain: &str, routes: &[&'static RoutableService]) -> String {
    let mut cfg = HAPROXY_HEADER.to_owned();
    cfg += &format!(
        "# Concierge-generated HTTPS mode: :80 redirects to :443, which terminates TLS\n\
         # with the leaf certificate `concierge tls enable`/`set-ca` maintains at\n\
         # {leaf_path}. Requests route by Host header to each service enabled via\n\
         # `concierge service enable` (see daemon/services/routes.rs), falling back to\n\
         # concierge itself. Manual edits survive only until the next `concierge tls\n\
         # enable`/`disable` or `concierge service enable`/`disable` call.\n\
         frontend concierge_http\n\
         \x20   bind *:80\n\
         \x20   redirect scheme https code 301\n\
         \n\
         frontend concierge_in\n\
         \x20   bind *:443 ssl crt {leaf_path}\n",
        leaf_path = leaf_path.display(),
    );
    for route in routes {
        cfg += &format!(
            "    acl route_{label} hdr(host) -i {label}.{domain}\n    use_backend {label} if route_{label}\n",
            label = route.label,
        );
    }
    cfg += "    default_backend concierge\n\nbackend concierge\n    server concierge1 127.0.0.1:8080 check\n";
    for route in routes {
        cfg += &format!(
            "\nbackend {label}\n    server {label}1 {addr} check\n",
            label = route.label,
            addr = route.backend_addr,
        );
    }
    cfg
}

const HAPROXY_HEADER: &str = "global\n\
    log /dev/log local0\n\
    log /dev/log local1 notice\n\
    stats socket /run/haproxy/admin.sock mode 660 level admin\n\
    stats timeout 30s\n\
    user  haproxy\n\
    group haproxy\n\
\n\
defaults\n\
    log     global\n\
    mode    http\n\
    option  httplog\n\
    option  dontlognull\n\
    timeout connect 5s\n\
    timeout client  30s\n\
    timeout server  30s\n\
    timeout http-request 10s\n\
\n\
# Concierge binds 0.0.0.0:8080 (see ../concierge/concierge/concierge.toml).\n";

fn write_atomic(path: &Path, content: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("/"));
    std::fs::create_dir_all(parent).map_err(io_err)?;
    let temp_path = parent.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("concierge-tls")
    ));
    std::fs::write(&temp_path, content).map_err(io_err)?;
    std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(mode)).map_err(io_err)?;
    std::fs::rename(&temp_path, path).map_err(io_err)
}

fn format_rfc3339(instant: OffsetDateTime) -> Result<String> {
    instant.format(&Rfc3339).map_err(|error| ServiceError::Other(error.into()))
}

fn cert_err(error: rcgen::Error) -> ServiceError {
    ServiceError::Other(anyhow::anyhow!("certificate error: {error}"))
}

fn io_err(error: std::io::Error) -> ServiceError {
    ServiceError::Other(error.into())
}

#[cfg(test)]
mod tests {
    use x509_parser::extensions::GeneralName;
    use x509_parser::pem::parse_x509_pem;

    use super::*;

    #[test]
    fn plain_cfg_has_no_tls() {
        let cfg = render_plain_cfg();
        assert!(cfg.contains("bind *:80"));
        assert!(cfg.contains("default_backend concierge"));
        assert!(!cfg.contains("ssl crt"));
    }

    #[test]
    fn tls_cfg_binds_443_with_leaf_path() {
        let cfg = render_tls_cfg(Path::new("/var/lib/foyer/tls/leaf/haproxy.pem"), "foyer.example", &[]);
        assert!(cfg.contains("bind *:443 ssl crt /var/lib/foyer/tls/leaf/haproxy.pem"));
        assert!(cfg.contains("redirect scheme https"));
        assert!(cfg.contains("default_backend concierge"));
    }

    #[test]
    fn tls_cfg_routes_enabled_services_by_subdomain() {
        let route = &ROUTABLE_SERVICES[0];
        let cfg = render_tls_cfg(Path::new("/leaf.pem"), "foyer.example", &[route]);
        assert!(cfg.contains(&format!("hdr(host) -i {}.foyer.example", route.label)));
        assert!(cfg.contains(&format!("use_backend {}", route.label)));
        assert!(cfg.contains(&format!("backend {}\n    server {}1 {}", route.label, route.label, route.backend_addr)));
    }

    #[test]
    fn ca_and_leaf_round_trip() {
        let ca = generate_ca().expect("generate CA");
        let leaf = issue_leaf("foyer.example", &ca.cert_pem, &ca.key_pem).expect("issue leaf");

        // The combined PEM haproxy's `crt` directive expects: cert then key.
        assert!(leaf.combined_pem.contains("BEGIN CERTIFICATE"));
        assert!(leaf.combined_pem.contains("BEGIN PRIVATE KEY"));

        let leaf_cert_pem = leaf.combined_pem.split("-----END CERTIFICATE-----").next().unwrap();
        let leaf_cert_pem = format!("{leaf_cert_pem}-----END CERTIFICATE-----\n");
        let (_, pem) = parse_x509_pem(leaf_cert_pem.as_bytes()).expect("parse leaf PEM");
        let cert = pem.parse_x509().expect("parse leaf DER");

        let sans = cert
            .subject_alternative_name()
            .expect("read SAN extension")
            .expect("SAN extension present");
        let dns_sans: Vec<&str> = sans
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                GeneralName::DNSName(name) => Some(*name),
                _ => None,
            })
            .collect();
        assert!(dns_sans.contains(&"foyer.example"), "leaf cert SAN should contain the requested domain");
        assert!(
            dns_sans.contains(&"*.foyer.example"),
            "leaf cert SAN should contain the wildcard subdomain for routable services"
        );
    }

    #[test]
    fn validate_ca_pair_accepts_matching_pair() {
        let ca = generate_ca().expect("generate CA");
        assert!(validate_ca_pair(&ca.cert_pem, &ca.key_pem).is_ok());
    }

    #[test]
    fn validate_ca_pair_rejects_mismatched_key() {
        let ca = generate_ca().expect("generate CA");
        let other = generate_ca().expect("generate other CA");
        assert!(validate_ca_pair(&ca.cert_pem, &other.key_pem).is_err());
    }
}
