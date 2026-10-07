//! Local mTLS certificate material, its cache, lifecycle, and platform trust
//! decisions.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use p256::ecdsa::SigningKey;
use p256::pkcs8::DecodePrivateKey;
use rustls::ServerConfig;
use x509_cert::Certificate;
use x509_cert::der::DecodePem;

use crate::cert_generation::{
    Issuer, build_ca_certificate, build_client_certificate, build_server_certificate,
    certificate_pem, generate_key, key_pem,
};
use crate::cert_validation::{
    certificate_is_issued_by, certificate_is_valid_now, certificate_matches_key,
    certificate_names_host, certificate_not_before, pem_contents,
};
use crate::lifecycle_events::{Event, Events};
use crate::{Error, Result};

/// How long after its validity starts a leaf certificate is renewed.
const LEAF_RENEWAL: Duration = Duration::from_hours(30 * 24);

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The certificate cache's file names in the app's state directory.
struct CertificatePaths;

impl CertificatePaths {
    const CA_CERT_PEM: &'static str = "ca.cert.pem";
    const CA_KEY_PEM: &'static str = "ca.key.pem";
    const SERVER_CERT_PEM: &'static str = "server.cert.pem";
    const SERVER_KEY_PEM: &'static str = "server.key.pem";
    const CLIENT_CERT_PEM: &'static str = "client.cert.pem";
    const CLIENT_KEY_PEM: &'static str = "client.key.pem";
}

/// Runtime certificate and key material, in PEM, for tokamak and platform
/// `WebViews`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CertificateBundle {
    /// CA certificate trusted by the local gateway for client authentication.
    pub ca_cert: String,
    /// CA private key used to issue replacement leaf certificates.
    pub ca_key: String,
    /// Server certificate used by the local TLS gateway.
    pub server_cert: String,
    /// Server private key used by the local TLS gateway.
    pub server_key: String,
    /// Client certificate.
    pub client_cert: String,
    /// Client private key.
    pub client_key: String,
}

impl CertificateBundle {
    /// Return a valid bundle for `host`, generating or renewing what is due.
    ///
    /// # Errors
    ///
    /// Returns an error when generation, cache loading, or cache writing fails.
    pub(crate) fn ensure(work_dir: &Path, host: &str, now: SystemTime) -> Result<Self> {
        let Ok(bundle) = Self::load_cached(work_dir) else {
            return Self::renew_or_generate(work_dir, host, now);
        };
        if !bundle.issuer_is_current(now) {
            return Self::generate_and_cache(work_dir, host, now);
        }
        if !bundle.cached_material_is_current(now)
            || !bundle.server_certificate_matches_host(host)
            || bundle.leaf_renewal_is_due(now)
        {
            let replacement = bundle.renew_leaves(host, now)?;
            replacement.write_leaves(work_dir)?;
            return Ok(replacement);
        }
        Ok(bundle)
    }

    fn renew_or_generate(work_dir: &Path, host: &str, now: SystemTime) -> Result<Self> {
        let Ok(issuer) = Self::load_issuer(work_dir) else {
            return Self::generate_and_cache(work_dir, host, now);
        };
        if !issuer.issuer_is_current(now) {
            return Self::generate_and_cache(work_dir, host, now);
        }
        let replacement = issuer.renew_leaves(host, now)?;
        replacement.write_all(work_dir)?;
        Ok(replacement)
    }

    fn generate_and_cache(work_dir: &Path, host: &str, now: SystemTime) -> Result<Self> {
        let bundle = Self::generate_at(host, now)?;
        bundle.write_all(work_dir)?;
        Ok(bundle)
    }

    /// Generate a self-signed local CA, an app-origin server certificate with
    /// loopback SANs, and a client-auth certificate, all ECDSA P-256/SHA-256.
    pub(crate) fn generate_at(host: &str, now: SystemTime) -> Result<Self> {
        let ca_key = generate_key();
        let ca_cert = build_ca_certificate(&ca_key, now)?;
        let authority = Self {
            ca_cert: certificate_pem(&ca_cert)?,
            ca_key: key_pem(&ca_key)?,
            ..Self::default()
        };
        authority.issue_leaves(&issuer(&ca_cert, &ca_key), host, now)
    }

    pub(crate) fn renew_leaves(&self, host: &str, now: SystemTime) -> Result<Self> {
        let ca_key = SigningKey::from_pkcs8_pem(&self.ca_key)?;
        let ca_cert = Certificate::from_pem(self.ca_cert.as_bytes())?;
        self.issue_leaves(&issuer(&ca_cert, &ca_key), host, now)
    }

    /// This bundle's authority with new server and client certificates that
    /// `issuer` issues.
    fn issue_leaves(&self, issuer: &Issuer<'_>, host: &str, now: SystemTime) -> Result<Self> {
        let server_key = generate_key();
        let client_key = generate_key();
        let server_cert = build_server_certificate(&server_key, issuer, host, now)?;
        let client_cert = build_client_certificate(&client_key, issuer, now)?;
        Ok(Self {
            ca_cert: self.ca_cert.clone(),
            ca_key: self.ca_key.clone(),
            server_cert: certificate_pem(&server_cert)?,
            server_key: key_pem(&server_key)?,
            client_cert: certificate_pem(&client_cert)?,
            client_key: key_pem(&client_key)?,
        })
    }

    /// Load all cached certificate files from a work directory.
    ///
    /// # Errors
    ///
    /// Returns an error when any expected certificate file cannot be read.
    pub fn load_cached(work_dir: impl AsRef<Path>) -> Result<Self> {
        let work_dir = work_dir.as_ref();
        let read = |name| fs::read_to_string(work_dir.join(name));
        Ok(Self {
            ca_cert: read(CertificatePaths::CA_CERT_PEM)?,
            ca_key: read(CertificatePaths::CA_KEY_PEM)?,
            server_cert: read(CertificatePaths::SERVER_CERT_PEM)?,
            server_key: read(CertificatePaths::SERVER_KEY_PEM)?,
            client_cert: read(CertificatePaths::CLIENT_CERT_PEM)?,
            client_key: read(CertificatePaths::CLIENT_KEY_PEM)?,
        })
    }

    pub(crate) fn load_issuer(work_dir: &Path) -> Result<Self> {
        Ok(Self {
            ca_cert: fs::read_to_string(work_dir.join(CertificatePaths::CA_CERT_PEM))?,
            ca_key: fs::read_to_string(work_dir.join(CertificatePaths::CA_KEY_PEM))?,
            ..Self::default()
        })
    }

    pub(crate) fn cached_material_is_current(&self, now: SystemTime) -> bool {
        self.issuer_is_current(now)
            && self.leaves_are_current(now)
            && certificate_matches_key(&self.server_cert, &self.server_key)
            && certificate_matches_key(&self.client_cert, &self.client_key)
            && certificate_is_issued_by(&self.server_cert, &self.ca_cert)
            && certificate_is_issued_by(&self.client_cert, &self.ca_cert)
    }

    pub(crate) fn issuer_is_current(&self, now: SystemTime) -> bool {
        certificate_is_valid_now(&self.ca_cert, now)
            && certificate_matches_key(&self.ca_cert, &self.ca_key)
    }

    fn leaves_are_current(&self, now: SystemTime) -> bool {
        [self.server_cert.as_str(), self.client_cert.as_str()]
            .into_iter()
            .all(|pem| certificate_is_valid_now(pem, now))
    }

    pub(crate) fn leaf_renewal_is_due(&self, now: SystemTime) -> bool {
        certificate_not_before(&self.server_cert)
            .is_none_or(|not_before| now >= not_before + LEAF_RENEWAL)
    }

    /// How long until the leaves are due for renewal.
    pub(crate) fn renewal_delay(&self, now: SystemTime) -> Duration {
        certificate_not_before(&self.server_cert).map_or(Duration::ZERO, |not_before| {
            (not_before + LEAF_RENEWAL)
                .duration_since(now)
                .unwrap_or_default()
        })
    }

    pub(crate) fn server_certificate_matches_host(&self, host: &str) -> bool {
        certificate_names_host(&self.server_cert, host)
    }

    /// Write all certificate files expected by tokamak and platform `WebViews`.
    ///
    /// # Errors
    ///
    /// Returns an error when the work directory cannot be created or files
    /// cannot be written.
    pub fn write_all(&self, work_dir: impl AsRef<Path>) -> Result<()> {
        let work_dir = work_dir.as_ref();
        fs::create_dir_all(work_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(work_dir, fs::Permissions::from_mode(0o700))?;
        }
        write_files(
            work_dir,
            self.authority_files().into_iter().chain(self.leaf_files()),
        )
    }

    pub(crate) fn write_leaves(&self, work_dir: &Path) -> Result<()> {
        write_files(work_dir, self.leaf_files())
    }

    fn authority_files(&self) -> [(&'static str, &str); 2] {
        [
            (CertificatePaths::CA_CERT_PEM, &self.ca_cert),
            (CertificatePaths::CA_KEY_PEM, &self.ca_key),
        ]
    }

    fn leaf_files(&self) -> [(&'static str, &str); 4] {
        [
            (CertificatePaths::SERVER_CERT_PEM, &self.server_cert),
            (CertificatePaths::SERVER_KEY_PEM, &self.server_key),
            (CertificatePaths::CLIENT_CERT_PEM, &self.client_cert),
            (CertificatePaths::CLIENT_KEY_PEM, &self.client_key),
        ]
    }
}

/// The issuer whose name is `ca_cert`'s subject and whose key is `ca_key`.
fn issuer<'a>(ca_cert: &Certificate, ca_key: &'a SigningKey) -> Issuer<'a> {
    Issuer {
        name: ca_cert.tbs_certificate().subject().clone(),
        key: ca_key,
    }
}

/// Write each of `files` into `work_dir` atomically, keys readable only by
/// the app.
fn write_files<'a>(
    work_dir: &Path,
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<()> {
    for (name, content) in files {
        let private = matches!(
            name,
            CertificatePaths::CA_KEY_PEM
                | CertificatePaths::SERVER_KEY_PEM
                | CertificatePaths::CLIENT_KEY_PEM
        );
        write_atomic(work_dir, name, content.as_bytes(), private)?;
    }
    Ok(())
}

fn write_atomic(directory: &Path, name: &str, content: &[u8], private: bool) -> Result<()> {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".{name}.{}.{counter}.tmp", std::process::id()));
    let result = write_temporary(&temporary, content, private)
        .and_then(|()| fs::rename(&temporary, directory.join(name)));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Error::from)
}

fn write_temporary(path: &Path, content: &[u8], private: bool) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(path)?;
    file.write_all(content)?;
    file.sync_all()
}

#[cfg(test)]
mod bundle_tests {
    use super::{CertificateBundle, CertificatePaths};
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
    use std::time::SystemTime;
    use time::{Duration, OffsetDateTime};
    use x509_parser::{
        certificate::X509Certificate,
        extensions::{GeneralName, ParsedExtension},
        parse_x509_certificate,
        pem::parse_x509_pem,
    };

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn ensure(work_dir: &std::path::Path) -> TestResult<CertificateBundle> {
        Ok(CertificateBundle::ensure(
            work_dir,
            "app.tokamak.local",
            SystemTime::now(),
        )?)
    }

    #[test]
    fn generates_then_reuses_a_cached_bundle() -> TestResult {
        let directory = tempfile::tempdir()?;

        let first = ensure(directory.path())?;
        assert_eq!(CertificateBundle::load_cached(directory.path())?, first);
        let second = ensure(directory.path())?;

        assert_eq!(first, second);
        Ok(())
    }

    /// Certificates that rcgen generated, as earlier releases did.
    fn rcgen_fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rcgen-certificates")
    }

    #[test]
    fn generates_certificates_like_those_rcgen_generated() -> TestResult {
        use sha2::{Digest, Sha256};

        let bundle = CertificateBundle::generate_at("app.tokamak.local", SystemTime::now())?;
        for (name, pem) in [
            ("ca", &bundle.ca_cert),
            ("server", &bundle.server_cert),
            ("client", &bundle.client_cert),
        ] {
            let fixture =
                std::fs::read_to_string(rcgen_fixture().join(format!("{name}.cert.pem")))?;
            let (generated_der, expected_der) =
                (decode_certificate(pem)?, decode_certificate(&fixture)?);
            let (_, generated) = parse_x509_certificate(&generated_der)?;
            let (_, expected) = parse_x509_certificate(&expected_der)?;
            let lifetime = |certificate: &X509Certificate<'_>| {
                certificate.validity().not_after.timestamp()
                    - certificate.validity().not_before.timestamp()
            };
            let extensions = |certificate: &X509Certificate<'_>| {
                let mut kinds: Vec<_> = certificate
                    .extensions()
                    .iter()
                    .map(|extension| (extension.oid.to_id_string(), extension.critical))
                    .collect();
                kinds.sort();
                kinds
            };
            let key_identifier = |certificate: &X509Certificate<'_>| {
                certificate.extensions().iter().find_map(|extension| {
                    match extension.parsed_extension() {
                        ParsedExtension::SubjectKeyIdentifier(identifier) => {
                            Some(identifier.0.to_vec())
                        }
                        _ => None,
                    }
                })
            };
            let derived_identifier = |certificate: &X509Certificate<'_>| {
                Sha256::digest(&certificate.public_key().subject_public_key.data)[..20].to_vec()
            };

            assert_eq!(extensions(&generated), extensions(&expected), "{name}");
            assert_eq!(generated.raw_serial(), expected.raw_serial(), "{name}");
            assert_eq!(
                generated.subject().to_string(),
                expected.subject().to_string(),
                "{name}"
            );
            assert_eq!(
                generated.issuer().to_string(),
                expected.issuer().to_string(),
                "{name}"
            );
            assert_eq!(lifetime(&generated), lifetime(&expected), "{name}");
            assert_eq!(
                generated.signature_algorithm.algorithm,
                expected.signature_algorithm.algorithm
            );
            assert_eq!(
                key_identifier(&generated),
                Some(derived_identifier(&generated)),
                "{name}"
            );
        }
        Ok(())
    }

    #[test]
    fn completes_a_mutual_tls_handshake_with_generated_certificates() -> TestResult {
        use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};

        let bundle = CertificateBundle::generate_at("app.tokamak.local", SystemTime::now())?;
        let server = crate::tls::server_config(
            bundle.server_cert.as_bytes(),
            bundle.server_key.as_bytes(),
            bundle.ca_cert.as_bytes(),
        )?;
        let mut roots = rustls::RootCertStore::empty();
        for authority in CertificateDer::pem_slice_iter(bundle.ca_cert.as_bytes()) {
            roots.add(authority?)?;
        }
        let chain = CertificateDer::pem_slice_iter(bundle.client_cert.as_bytes())
            .collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_slice(bundle.client_key.as_bytes())?;
        let client = rustls::ClientConfig::builder_with_provider(crate::tls::provider())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)?;
        let host = ServerName::try_from("app.tokamak.local")?;
        let mut client = rustls::ClientConnection::new(std::sync::Arc::new(client), host)?;
        let mut server = rustls::ServerConnection::new(server)?;

        for _ in 0..8 {
            let mut bytes = Vec::new();
            client.write_tls(&mut bytes)?;
            server.read_tls(&mut bytes.as_slice())?;
            server.process_new_packets()?;
            bytes.clear();
            server.write_tls(&mut bytes)?;
            client.read_tls(&mut bytes.as_slice())?;
            client.process_new_packets()?;
        }

        assert!(!client.is_handshaking() && !server.is_handshaking());
        assert!(
            server
                .peer_certificates()
                .is_some_and(|chain| !chain.is_empty())
        );
        Ok(())
    }

    #[test]
    fn renews_a_server_certificate_another_authority_signed() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        let foreign = CertificateBundle::generate_at("app.tokamak.local", SystemTime::now())?;
        for (name, content) in [
            (CertificatePaths::SERVER_CERT_PEM, &foreign.server_cert),
            (CertificatePaths::SERVER_KEY_PEM, &foreign.server_key),
        ] {
            std::fs::write(directory.path().join(name), content)?;
        }

        let second = ensure(directory.path())?;

        assert_ne!(second.server_cert, foreign.server_cert);
        assert_eq!(second.ca_cert, first.ca_cert);
        Ok(())
    }

    #[test]
    fn leaves_are_due_for_renewal_once_the_delay_elapses() -> TestResult {
        let issued = SystemTime::now();
        let bundle = CertificateBundle::generate_at("app.tokamak.local", issued)?;
        let now = issued
            + std::time::Duration::from_hours(24 * 10)
            + std::time::Duration::from_nanos(123_456_789);

        let delay = bundle.renewal_delay(now);

        assert!(bundle.leaf_renewal_is_due(now + delay));
        assert!(!bundle.leaf_renewal_is_due(now + delay - std::time::Duration::from_secs(1)));
        Ok(())
    }

    #[test]
    fn keeps_and_renews_a_bundle_that_rcgen_generated() -> TestResult {
        // Earlier releases cached these; installs keep them across updates.
        let directory = tempfile::tempdir()?;
        for entry in std::fs::read_dir(rcgen_fixture())? {
            let entry = entry?;
            std::fs::copy(entry.path(), directory.path().join(entry.file_name()))?;
        }
        let cached = CertificateBundle::load_cached(directory.path())?;
        let issued = crate::cert_validation::certificate_not_before(&cached.server_cert)
            .ok_or("fixture server certificate has no validity")?;
        let day = std::time::Duration::from_hours(24);

        let kept =
            CertificateBundle::ensure(directory.path(), "app.tokamak.local", issued + day * 2)?;
        let renewed =
            CertificateBundle::ensure(directory.path(), "app.tokamak.local", issued + day * 32)?;

        assert_eq!(kept, cached);
        assert_eq!(renewed.ca_cert, cached.ca_cert);
        assert_ne!(renewed.server_cert, cached.server_cert);
        assert!(renewed.cached_material_is_current(issued + day * 33));
        Ok(())
    }

    #[test]
    fn generates_platform_certificate_material() -> TestResult {
        let bundle = CertificateBundle::generate_at("app.tokamak.local", SystemTime::now())?;
        assert_pem_material(&bundle);

        let ca_der = decode_certificate(&bundle.ca_cert)?;
        let server_der = decode_certificate(&bundle.server_cert)?;
        let client_der = decode_certificate(&bundle.client_cert)?;
        let (_, ca) = parse_x509_certificate(&ca_der)?;
        let (_, server) = parse_x509_certificate(&server_der)?;
        let (_, client) = parse_x509_certificate(&client_der)?;

        assert_certificate_chain(&ca, &server, &client)?;
        assert_server_names(&server)?;
        assert_certificate_usages(&ca, &server, &client)?;
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn writes_private_material_with_private_permissions() -> TestResult {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir()?;
        ensure(directory.path())?;

        for name in [
            CertificatePaths::SERVER_KEY_PEM,
            CertificatePaths::CA_KEY_PEM,
            CertificatePaths::CLIENT_KEY_PEM,
        ] {
            let mode = std::fs::metadata(directory.path().join(name))?
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name}");
        }
        Ok(())
    }

    #[test]
    fn recovers_from_a_partial_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let stale = directory.path().join(CertificatePaths::SERVER_CERT_PEM);
        std::fs::write(&stale, "not a certificate")?;

        let bundle = ensure(directory.path())?;

        assert_eq!(CertificateBundle::load_cached(directory.path())?, bundle);
        assert_eq!(std::fs::read_to_string(stale)?, bundle.server_cert);
        Ok(())
    }

    #[test]
    fn regenerates_an_expired_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        let expired = expired_certificate_pem()?;
        std::fs::write(
            directory.path().join(CertificatePaths::SERVER_CERT_PEM),
            expired.as_bytes(),
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.server_cert, expired);
        assert_ne!(second.server_cert, first.server_cert);
        assert_eq!(second.ca_cert, first.ca_cert);
        Ok(())
    }

    #[test]
    fn rotates_leaves_without_rotating_the_issuer() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;

        let second = CertificateBundle::ensure(
            directory.path(),
            "app.tokamak.local",
            SystemTime::now() + std::time::Duration::from_hours(31 * 24),
        )?;

        assert_eq!(second.ca_cert, first.ca_cert);
        assert_eq!(second.ca_key, first.ca_key);
        assert_ne!(second.server_cert, first.server_cert);
        assert_ne!(second.client_cert, first.client_cert);
        Ok(())
    }

    #[test]
    fn recovers_missing_leaves_without_rotating_the_issuer() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::remove_file(directory.path().join(CertificatePaths::CLIENT_CERT_PEM))?;

        let second = ensure(directory.path())?;

        assert_eq!(second.ca_cert, first.ca_cert);
        assert_ne!(second.client_cert, first.client_cert);
        assert_eq!(CertificateBundle::load_cached(directory.path())?, second);
        Ok(())
    }

    #[test]
    fn regenerates_a_corrupt_authority_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CA_CERT_PEM),
            "not a certificate",
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.ca_cert, first.ca_cert);
        assert_eq!(CertificateBundle::load_cached(directory.path())?, second);
        Ok(())
    }

    #[test]
    fn regenerates_a_corrupt_client_key_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CLIENT_KEY_PEM),
            "not a key",
        )?;

        let second = ensure(directory.path())?;

        assert_eq!(second.ca_cert, first.ca_cert);
        assert_ne!(second.client_key, first.client_key);
        assert!(second.cached_material_is_current(SystemTime::now()));
        Ok(())
    }

    #[test]
    fn regenerates_a_mismatched_authority_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        let foreign = CertificateBundle::generate_at("app.tokamak.local", SystemTime::now())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CA_CERT_PEM),
            &foreign.ca_cert,
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.ca_cert, foreign.ca_cert);
        assert_ne!(second.ca_cert, first.ca_cert);
        Ok(())
    }

    #[test]
    fn stores_only_pem_material() -> TestResult {
        let directory = tempfile::tempdir()?;
        ensure(directory.path())?;

        let mut names = std::fs::read_dir(directory.path())?
            .map(|entry| Ok(entry?.file_name().into_string().map_err(|_| "file name")?))
            .collect::<TestResult<Vec<_>>>()?;
        names.sort();

        assert_eq!(
            names,
            [
                "ca.cert.pem",
                "ca.key.pem",
                "client.cert.pem",
                "client.key.pem",
                "server.cert.pem",
                "server.key.pem"
            ]
        );
        Ok(())
    }

    fn expired_certificate_pem() -> TestResult<String> {
        let key = KeyPair::generate()?;
        let now = OffsetDateTime::now_utc();
        let mut params = CertificateParams::default();
        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, "expired.localhost");
        params.distinguished_name = distinguished_name;
        params.not_before = now - Duration::days(10);
        params.not_after = now - Duration::days(1);
        Ok(params.self_signed(&key)?.pem())
    }

    fn assert_pem_material(bundle: &CertificateBundle) {
        const CERTIFICATE: &str = "-----BEGIN CERTIFICATE-----";
        const PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----";

        assert!(bundle.ca_cert.starts_with(CERTIFICATE));
        assert!(bundle.server_cert.starts_with(CERTIFICATE));
        assert!(bundle.server_key.starts_with(PRIVATE_KEY));
        assert!(bundle.ca_key.starts_with(PRIVATE_KEY));
        assert!(bundle.client_cert.starts_with(CERTIFICATE));
        assert!(bundle.client_key.starts_with(PRIVATE_KEY));
    }

    fn assert_certificate_chain(
        ca: &X509Certificate<'_>,
        server: &X509Certificate<'_>,
        client: &X509Certificate<'_>,
    ) -> TestResult {
        assert_eq!(common_name(ca)?, "tokamak local ca");
        assert_eq!(common_name(server)?, "app.tokamak.local");
        assert_eq!(common_name(client)?, "tokamak client");
        assert!(ca.validity().is_valid());
        assert!(server.validity().is_valid());
        assert!(client.validity().is_valid());
        ca.verify_signature(None)?;
        assert_eq!(server.issuer(), ca.subject());
        assert_eq!(client.issuer(), ca.subject());
        server.verify_signature(Some(ca.public_key()))?;
        client.verify_signature(Some(ca.public_key()))?;
        Ok(())
    }

    fn assert_server_names(server: &X509Certificate<'_>) -> TestResult {
        let names = server
            .extensions()
            .iter()
            .find_map(|extension| {
                let ParsedExtension::SubjectAlternativeName(san) = extension.parsed_extension()
                else {
                    return None;
                };
                Some(&san.general_names)
            })
            .ok_or("server certificate must include SANs")?;
        assert!(
            names
                .iter()
                .any(|name| matches!(name, GeneralName::DNSName("localhost")))
        );
        assert!(
            names
                .iter()
                .any(|name| matches!(name, GeneralName::DNSName("app.tokamak.local")))
        );
        assert!(
            names.iter().any(
                |name| matches!(name, GeneralName::IPAddress(value) if *value == [127, 0, 0, 1])
            )
        );
        Ok(())
    }

    fn assert_certificate_usages(
        ca: &X509Certificate<'_>,
        server: &X509Certificate<'_>,
        client: &X509Certificate<'_>,
    ) -> TestResult {
        assert!(
            ca.basic_constraints()?
                .ok_or("CA basic constraints are missing")?
                .value
                .ca
        );
        let ca_usage = ca.key_usage()?.ok_or("CA key usage is missing")?.value;
        assert!(ca_usage.key_cert_sign());
        assert!(ca_usage.crl_sign());

        let server_usage = server
            .extended_key_usage()?
            .ok_or("server extended key usage is missing")?
            .value;
        assert!(server_usage.server_auth);
        assert!(!server_usage.client_auth);

        let client_usage = client
            .extended_key_usage()?
            .ok_or("client extended key usage is missing")?
            .value;
        assert!(client_usage.client_auth);
        assert!(!client_usage.server_auth);
        Ok(())
    }

    fn common_name(certificate: &X509Certificate<'_>) -> TestResult<String> {
        let entry = certificate
            .subject()
            .iter_common_name()
            .next()
            .ok_or("certificate subject is missing a common name")?;
        Ok(entry.as_str()?.to_owned())
    }

    fn decode_certificate(pem: &str) -> TestResult<Vec<u8>> {
        Ok(parse_x509_pem(pem.as_bytes())?.1.contents)
    }
}

const RETRY_DELAY: Duration = Duration::from_hours(1);

type Shared = Arc<RwLock<CertificateBundle>>;

/// A TLS authentication challenge raised by a platform `WebView`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Challenge<'a> {
    /// The platform is deciding whether to trust the gateway's certificate.
    ServerTrust {
        /// Host the connection was made to.
        host: &'a str,
    },
    /// The gateway asked the platform for a client certificate.
    ClientCertificate {
        /// Host the connection was made to.
        host: &'a str,
        /// How many times this challenge has already failed.
        previous_failures: usize,
    },
}

/// How a shell answers a [`Challenge`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    /// Not a tokamak connection. Let the platform decide.
    PerformDefault,
    /// A tokamak connection that tokamak cannot authenticate. Refuse it.
    Cancel,
    /// Trust the certificate only where it chains to this authority, in DER.
    TrustAuthority(Vec<u8>),
    /// Present this client identity.
    PresentIdentity {
        /// Client certificate, in DER.
        certificate: Vec<u8>,
        /// Client private key, in PKCS#8 DER.
        private_key: Vec<u8>,
    },
}

/// Certificate material for the app's local mTLS gateway.
///
/// Certificates are generated on first use, cached in the app's state
/// directory, and renewed in the background before they expire.
#[derive(Debug)]
pub struct Certificates {
    state_dir: PathBuf,
    host: String,
    current: Shared,
    server_config: Mutex<Option<Arc<ServerConfig>>>,
}

impl Certificates {
    pub(crate) fn start(state_dir: PathBuf, host: String) -> Result<Self> {
        let bundle = CertificateBundle::ensure(&state_dir, &host, SystemTime::now())?;
        Ok(Self {
            state_dir,
            host,
            current: Arc::new(RwLock::new(bundle)),
            server_config: Mutex::new(None),
        })
    }

    /// The gateway's TLS configuration for the current server certificate, built once per renewal.
    pub(crate) fn server_config(&self) -> Result<Arc<ServerConfig>> {
        let mut cached = self
            .server_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(config) = &*cached {
            return Ok(Arc::clone(config));
        }
        let bundle = self
            .current
            .read()
            .map_err(|_| Error::CertificatesUnavailable)?;
        let config = crate::tls::server_config(
            bundle.server_cert.as_bytes(),
            bundle.server_key.as_bytes(),
            bundle.ca_cert.as_bytes(),
        )?;
        *cached = Some(Arc::clone(&config));
        Ok(config)
    }

    pub(crate) fn start_renewal(self: &Arc<Self>, events: Events) -> Renewal {
        Renewal::start(Arc::clone(self), events)
    }

    /// Renew any certificate that is due.
    pub(crate) fn refresh(&self) -> Result<()> {
        let bundle = CertificateBundle::ensure(&self.state_dir, &self.host, SystemTime::now())?;
        {
            let mut held = self
                .current
                .write()
                .map_err(|_| Error::CertificatesUnavailable)?;
            *held = bundle;
        }
        // Cleared after the bundle lock is released: `server_config` takes them in the other order.
        *self
            .server_config
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }

    fn next_delay(&self) -> Duration {
        self.current
            .read()
            .map_or(RETRY_DELAY, |held| held.renewal_delay(SystemTime::now()))
    }

    /// Decide how a shell should answer a platform TLS challenge.
    #[must_use]
    pub fn decide(&self, challenge: &Challenge<'_>) -> Decision {
        let (Challenge::ServerTrust { host } | Challenge::ClientCertificate { host, .. }) =
            *challenge;
        if !same_dns_host(host, &self.host) {
            return Decision::PerformDefault;
        }
        let Ok(current) = self.current.read() else {
            return Decision::Cancel;
        };
        match *challenge {
            Challenge::ServerTrust { .. } => {
                pem_contents(&current.ca_cert).map_or(Decision::Cancel, Decision::TrustAuthority)
            }
            Challenge::ClientCertificate {
                previous_failures, ..
            } => client_identity(&current, previous_failures),
        }
    }

    /// Return whether `certificate` is the current server certificate for `host`.
    #[must_use]
    pub fn trusts_server_certificate(&self, host: &str, certificate: &str) -> bool {
        if !same_dns_host(host, &self.host) {
            return false;
        }
        let Ok(current) = self.current.read() else {
            return false;
        };
        pem_contents(certificate) == pem_contents(&current.server_cert)
    }
}

fn client_identity(current: &CertificateBundle, previous_failures: usize) -> Decision {
    if previous_failures > 0 {
        return Decision::Cancel;
    }
    match (
        pem_contents(&current.client_cert),
        pem_contents(&current.client_key),
    ) {
        (Some(certificate), Some(private_key)) => Decision::PresentIdentity {
            certificate,
            private_key,
        },
        _ => Decision::Cancel,
    }
}

fn same_dns_host(left: &str, right: &str) -> bool {
    left.trim_end_matches('.')
        .eq_ignore_ascii_case(right.trim_end_matches('.'))
}

#[derive(Debug)]
pub(crate) struct Renewal {
    stop: Sender<()>,
    task: Option<JoinHandle<()>>,
}

impl Renewal {
    fn start(certificates: Arc<Certificates>, events: Events) -> Self {
        let (stop, stopped) = mpsc::channel();
        let task = thread::spawn(move || {
            let mut retrying = false;
            loop {
                let delay = if retrying {
                    RETRY_DELAY
                } else {
                    certificates.next_delay()
                };
                if stopped.recv_timeout(delay).is_ok() {
                    break;
                }
                match certificates.refresh() {
                    Ok(()) => {
                        retrying = false;
                        events.emit(Event::CertificatesRenewed);
                    }
                    Err(error) => {
                        retrying = true;
                        events.emit(Event::Failed {
                            message: format!("certificate renewal failed: {error}"),
                        });
                    }
                }
            }
        });
        Self {
            stop,
            task: Some(task),
        }
    }
}

impl Drop for Renewal {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}

#[cfg(test)]
mod certificate_tests {
    use std::sync::Arc;

    use super::{CertificatePaths, Certificates, Challenge, Decision};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    fn certificates(directory: &std::path::Path) -> TestResult<Certificates> {
        Ok(Certificates::start(
            directory.to_path_buf(),
            "app.tokamak.local".to_owned(),
        )?)
    }

    #[test]
    fn reuses_the_tls_server_config_until_certificates_refresh() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;

        let first = certificates.server_config()?;
        let again = certificates.server_config()?;
        assert!(Arc::ptr_eq(&first, &again));

        certificates.refresh()?;
        let renewed = certificates.server_config()?;
        assert!(!Arc::ptr_eq(&first, &renewed));
        Ok(())
    }

    #[test]
    fn pins_the_generated_authority_for_the_app_host() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;

        let decision = certificates.decide(&Challenge::ServerTrust {
            host: "APP.TOKAMAK.LOCAL.",
        });

        assert!(matches!(decision, Decision::TrustAuthority(der) if !der.is_empty()));
        Ok(())
    }

    #[test]
    fn presents_a_client_identity_for_the_app_host() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;

        let decision = certificates.decide(&Challenge::ClientCertificate {
            host: "app.tokamak.local",
            previous_failures: 0,
        });

        let Decision::PresentIdentity {
            certificate,
            private_key,
        } = decision
        else {
            return Err(std::io::Error::other("expected a client identity").into());
        };
        assert!(!certificate.is_empty());
        assert!(!private_key.is_empty());
        Ok(())
    }

    #[test]
    fn leaves_other_hosts_to_the_platform() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;

        assert_eq!(
            certificates.decide(&Challenge::ServerTrust {
                host: "example.com"
            }),
            Decision::PerformDefault
        );
        assert_eq!(
            certificates.decide(&Challenge::ClientCertificate {
                host: "example.com",
                previous_failures: 0,
            }),
            Decision::PerformDefault
        );
        Ok(())
    }

    #[test]
    fn refuses_a_client_certificate_that_already_failed() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;

        assert_eq!(
            certificates.decide(&Challenge::ClientCertificate {
                host: "app.tokamak.local",
                previous_failures: 1,
            }),
            Decision::Cancel
        );
        Ok(())
    }

    #[test]
    fn trusts_only_the_current_server_certificate_for_the_app_host() -> TestResult {
        let directory = tempfile::tempdir()?;
        let certificates = certificates(directory.path())?;
        let server =
            std::fs::read_to_string(directory.path().join(CertificatePaths::SERVER_CERT_PEM))?;
        let certificate = server
            .split("-----END CERTIFICATE-----")
            .next()
            .ok_or("server certificate is missing")?;
        let certificate = format!("{certificate}-----END CERTIFICATE-----\n");

        assert!(certificates.trusts_server_certificate("app.tokamak.local", &certificate));
        assert!(!certificates.trusts_server_certificate("example.com", &certificate));
        assert!(!certificates.trusts_server_certificate("app.tokamak.local", "invalid"));
        Ok(())
    }
}
