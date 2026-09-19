//! Local mTLS certificate material, lifecycle, and platform trust decisions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration as StdDuration;

use rcgen::{Issuer, KeyPair};
use rustls::ServerConfig;
use time::{Duration, OffsetDateTime};
use x509_parser::extensions::{GeneralName, ParsedExtension};

use crate::cert_generation::{
    CertificatePaths, build_ca_certificate, build_client_certificate, build_server_certificate,
    is_private_key, remove_if_exists, server_identity, write_atomic,
};
use crate::cert_validation::{
    certificate_der, certificate_der_matches_pem, certificate_is_issued_by,
    certificate_is_valid_now, certificate_matches_key, certificate_not_before, key_matches_der,
    with_certificate,
};
use crate::lifecycle_events::{Event, Events};
use crate::{Error, Result};

const LEAF_RENEWAL_DAYS: i64 = 30;

/// Runtime certificate and key material needed by tokamak and platform `WebViews`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateBundle {
    /// CA certificate PEM trusted by the local gateway for client authentication.
    pub ca_cert_pem: String,
    /// CA private key PEM used to issue replacement leaf certificates.
    pub ca_key_pem: String,
    /// Server certificate PEM used by the local TLS gateway.
    pub server_cert_pem: String,
    /// Server private key PEM used by the local TLS gateway.
    pub server_key_pem: String,
    /// Server certificate and key PEM consumed atomically by the local gateway.
    pub server_identity_pem: String,
    /// Client certificate PEM.
    pub client_cert_pem: String,
    /// Client private key PEM.
    pub client_key_pem: String,
    /// Client private key PKCS#8 DER for in-memory platform credentials.
    pub client_key_der: Vec<u8>,
    /// CA certificate DER for platform trust APIs.
    pub ca_cert_der: Vec<u8>,
}

impl CertificateBundle {
    /// Return a valid bundle for `host`, generating or renewing what is due.
    ///
    /// # Errors
    ///
    /// Returns an error when generation, cache loading, or cache writing fails.
    pub(crate) fn ensure(work_dir: &Path, host: &str, now: OffsetDateTime) -> Result<Self> {
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

    fn renew_or_generate(work_dir: &Path, host: &str, now: OffsetDateTime) -> Result<Self> {
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

    fn generate_and_cache(work_dir: &Path, host: &str, now: OffsetDateTime) -> Result<Self> {
        let bundle = Self::generate_at(host, now)?;
        bundle.write_all(work_dir)?;
        Ok(bundle)
    }

    /// The client certificate in DER, for platform credential APIs.
    pub(crate) fn client_certificate_der(&self) -> Option<Vec<u8>> {
        certificate_der(&self.client_cert_pem)
    }

    /// Generate a self-signed local CA, an app-origin server certificate with
    /// loopback SANs, and a client-auth certificate, all ECDSA P-256/SHA-256.
    pub(crate) fn generate_at(host: &str, now: OffsetDateTime) -> Result<Self> {
        let ca_key = KeyPair::generate()?;
        let server_key = KeyPair::generate()?;
        let client_key = KeyPair::generate()?;

        let (ca_params, ca_cert) = build_ca_certificate(&ca_key, now)?;
        let ca_issuer = Issuer::from_params(&ca_params, &ca_key);
        let server_cert = build_server_certificate(&server_key, &ca_issuer, host, now)?;
        let client_cert = build_client_certificate(&client_key, &ca_issuer, now)?;
        let server_cert_pem = server_cert.pem();
        let server_key_pem = server_key.serialize_pem();
        let client_cert_pem = client_cert.pem();
        let client_key_pem = client_key.serialize_pem();

        Ok(Self {
            ca_cert_pem: ca_cert.pem(),
            ca_key_pem: ca_key.serialize_pem(),
            server_identity_pem: server_identity(&server_cert_pem, &server_key_pem),
            server_cert_pem,
            server_key_pem,
            client_cert_pem,
            client_key_pem,
            client_key_der: client_key.serialized_der().to_vec(),
            ca_cert_der: ca_cert.der().to_vec(),
        })
    }

    pub(crate) fn renew_leaves(&self, host: &str, now: OffsetDateTime) -> Result<Self> {
        let ca_key = KeyPair::from_pem(&self.ca_key_pem)?;
        let ca_issuer = Issuer::from_ca_cert_pem(&self.ca_cert_pem, ca_key)?;
        let server_key = KeyPair::generate()?;
        let client_key = KeyPair::generate()?;
        let server_cert = build_server_certificate(&server_key, &ca_issuer, host, now)?;
        let client_cert = build_client_certificate(&client_key, &ca_issuer, now)?;

        let server_cert_pem = server_cert.pem();
        let server_key_pem = server_key.serialize_pem();
        Ok(Self {
            ca_cert_pem: self.ca_cert_pem.clone(),
            ca_key_pem: self.ca_key_pem.clone(),
            server_identity_pem: server_identity(&server_cert_pem, &server_key_pem),
            server_cert_pem,
            server_key_pem,
            client_cert_pem: client_cert.pem(),
            client_key_pem: client_key.serialize_pem(),
            client_key_der: client_key.serialized_der().to_vec(),
            ca_cert_der: self.ca_cert_der.clone(),
        })
    }

    /// Load all cached certificate files from a work directory.
    ///
    /// # Errors
    ///
    /// Returns an error when any expected certificate file cannot be read.
    pub fn load_cached(work_dir: impl AsRef<Path>) -> Result<Self> {
        let work_dir = work_dir.as_ref();
        if !CertificatePaths::all_exist(work_dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "certificate cache is incomplete",
            )
            .into());
        }
        Ok(Self {
            ca_cert_pem: fs::read_to_string(work_dir.join(CertificatePaths::CA_CERT_PEM))?,
            ca_key_pem: fs::read_to_string(work_dir.join(CertificatePaths::CA_KEY_PEM))?,
            server_cert_pem: fs::read_to_string(work_dir.join(CertificatePaths::SERVER_CERT_PEM))?,
            server_key_pem: fs::read_to_string(work_dir.join(CertificatePaths::SERVER_KEY_PEM))?,
            server_identity_pem: fs::read_to_string(
                work_dir.join(CertificatePaths::SERVER_IDENTITY_PEM),
            )?,
            client_cert_pem: fs::read_to_string(work_dir.join(CertificatePaths::CLIENT_CERT_PEM))?,
            client_key_pem: fs::read_to_string(work_dir.join(CertificatePaths::CLIENT_KEY_PEM))?,
            client_key_der: fs::read(work_dir.join(CertificatePaths::CLIENT_KEY_DER))?,
            ca_cert_der: fs::read(work_dir.join(CertificatePaths::CA_CERT_DER))?,
        })
    }

    pub(crate) fn load_issuer(work_dir: &Path) -> Result<Self> {
        Ok(Self {
            ca_cert_pem: fs::read_to_string(work_dir.join(CertificatePaths::CA_CERT_PEM))?,
            ca_key_pem: fs::read_to_string(work_dir.join(CertificatePaths::CA_KEY_PEM))?,
            server_cert_pem: String::new(),
            server_key_pem: String::new(),
            server_identity_pem: String::new(),
            client_cert_pem: String::new(),
            client_key_pem: String::new(),
            client_key_der: Vec::new(),
            ca_cert_der: fs::read(work_dir.join(CertificatePaths::CA_CERT_DER))?,
        })
    }

    pub(crate) fn cached_material_is_current(&self, now: OffsetDateTime) -> bool {
        self.issuer_is_current(now)
            && self.leaves_are_current(now)
            && certificate_matches_key(&self.server_cert_pem, &self.server_key_pem)
            && certificate_matches_key(&self.client_cert_pem, &self.client_key_pem)
            && key_matches_der(&self.client_key_pem, &self.client_key_der)
            && certificate_is_issued_by(&self.server_cert_pem, &self.ca_cert_pem)
            && certificate_is_issued_by(&self.client_cert_pem, &self.ca_cert_pem)
            && self.server_identity_pem
                == server_identity(&self.server_cert_pem, &self.server_key_pem)
    }

    pub(crate) fn issuer_is_current(&self, now: OffsetDateTime) -> bool {
        certificate_is_valid_now(&self.ca_cert_pem, now)
            && certificate_der_matches_pem(&self.ca_cert_pem, &self.ca_cert_der)
            && certificate_matches_key(&self.ca_cert_pem, &self.ca_key_pem)
    }

    fn leaves_are_current(&self, now: OffsetDateTime) -> bool {
        [self.server_cert_pem.as_str(), self.client_cert_pem.as_str()]
            .into_iter()
            .all(|pem| certificate_is_valid_now(pem, now))
    }

    pub(crate) fn leaf_renewal_is_due(&self, now: OffsetDateTime) -> bool {
        certificate_not_before(&self.server_cert_pem)
            .is_none_or(|not_before| now >= not_before + Duration::days(LEAF_RENEWAL_DAYS))
    }

    pub(crate) fn renewal_delay(&self, now: OffsetDateTime) -> std::time::Duration {
        let Some(not_before) = certificate_not_before(&self.server_cert_pem) else {
            return std::time::Duration::ZERO;
        };
        let seconds = u64::try_from(
            (not_before + Duration::days(LEAF_RENEWAL_DAYS) - now)
                .whole_seconds()
                .max(0),
        )
        .unwrap_or_default();
        std::time::Duration::from_secs(seconds)
    }

    pub(crate) fn server_certificate_matches_host(&self, host: &str) -> bool {
        with_certificate(&self.server_cert_pem, |certificate| {
            certificate.extensions().iter().any(|extension| {
                let ParsedExtension::SubjectAlternativeName(san) = extension.parsed_extension()
                else {
                    return false;
                };
                san.general_names
                    .iter()
                    .any(|name| matches!(name, GeneralName::DNSName(value) if *value == host))
            })
        })
        .unwrap_or(false)
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
        crate::cert_generation::set_directory_permissions(work_dir)?;
        write_files(
            work_dir,
            self.authority_files().into_iter().chain(self.leaf_files()),
        )
    }

    pub(crate) fn write_leaves(&self, work_dir: &Path) -> Result<()> {
        write_files(work_dir, self.leaf_files())
    }

    fn authority_files(&self) -> [(&str, &[u8]); 3] {
        [
            (CertificatePaths::CA_CERT_PEM, self.ca_cert_pem.as_bytes()),
            (CertificatePaths::CA_KEY_PEM, self.ca_key_pem.as_bytes()),
            (CertificatePaths::CA_CERT_DER, self.ca_cert_der.as_slice()),
        ]
    }

    fn leaf_files(&self) -> [(&str, &[u8]); 6] {
        [
            (
                CertificatePaths::SERVER_CERT_PEM,
                self.server_cert_pem.as_bytes(),
            ),
            (
                CertificatePaths::SERVER_KEY_PEM,
                self.server_key_pem.as_bytes(),
            ),
            (
                CertificatePaths::SERVER_IDENTITY_PEM,
                self.server_identity_pem.as_bytes(),
            ),
            (
                CertificatePaths::CLIENT_CERT_PEM,
                self.client_cert_pem.as_bytes(),
            ),
            (
                CertificatePaths::CLIENT_KEY_PEM,
                self.client_key_pem.as_bytes(),
            ),
            (
                CertificatePaths::CLIENT_KEY_DER,
                self.client_key_der.as_slice(),
            ),
        ]
    }
}

/// Write `files` into `work_dir`, clearing the completeness marker first and
/// restoring it once every file is in place.
fn write_files<'a>(
    work_dir: &Path,
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<()> {
    remove_if_exists(work_dir.join(CertificatePaths::CACHE_MARKER))?;
    for (name, content) in files {
        write_atomic(work_dir, name, content, is_private_key(name))?;
    }
    write_atomic(work_dir, CertificatePaths::CACHE_MARKER, &[], true)
}

#[cfg(test)]
mod bundle_tests {
    use super::{CertificateBundle, CertificatePaths};
    use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair};
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
            OffsetDateTime::now_utc(),
        )?)
    }

    #[test]
    fn generates_then_reuses_a_cached_bundle() -> TestResult {
        let directory = tempfile::tempdir()?;

        let first = ensure(directory.path())?;
        assert!(CertificatePaths::all_exist(directory.path()));
        let second = ensure(directory.path())?;

        assert_eq!(first.ca_cert_der, second.ca_cert_der);
        Ok(())
    }

    #[test]
    fn generates_platform_certificate_material() -> TestResult {
        let bundle =
            CertificateBundle::generate_at("app.tokamak.local", OffsetDateTime::now_utc())?;
        assert_pem_material(&bundle);

        let ca_der = decode_certificate(&bundle.ca_cert_pem)?;
        let server_der = decode_certificate(&bundle.server_cert_pem)?;
        let client_der = decode_certificate(&bundle.client_cert_pem)?;
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
            CertificatePaths::SERVER_IDENTITY_PEM,
            CertificatePaths::CLIENT_KEY_PEM,
            CertificatePaths::CLIENT_KEY_DER,
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

        assert!(CertificatePaths::all_exist(directory.path()));
        assert_eq!(std::fs::read_to_string(stale)?, bundle.server_cert_pem);
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

        assert_ne!(second.server_cert_pem, expired);
        assert_ne!(second.server_cert_pem, first.server_cert_pem);
        assert_eq!(second.ca_cert_pem, first.ca_cert_pem);
        Ok(())
    }

    #[test]
    fn rotates_leaves_without_rotating_the_issuer() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;

        let second = CertificateBundle::ensure(
            directory.path(),
            "app.tokamak.local",
            OffsetDateTime::now_utc() + Duration::days(31),
        )?;

        assert_eq!(second.ca_cert_pem, first.ca_cert_pem);
        assert_eq!(second.ca_key_pem, first.ca_key_pem);
        assert_ne!(second.server_cert_pem, first.server_cert_pem);
        assert_ne!(second.client_cert_pem, first.client_cert_pem);
        assert!(second.server_identity_pem.contains(&second.server_cert_pem));
        assert!(second.server_identity_pem.contains(&second.server_key_pem));
        Ok(())
    }

    #[test]
    fn recovers_missing_leaves_without_rotating_the_issuer() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::remove_file(directory.path().join(CertificatePaths::CLIENT_CERT_PEM))?;

        let second = ensure(directory.path())?;

        assert_eq!(second.ca_cert_pem, first.ca_cert_pem);
        assert_ne!(second.client_cert_pem, first.client_cert_pem);
        assert!(CertificatePaths::all_exist(directory.path()));
        Ok(())
    }

    #[test]
    fn regenerates_a_corrupt_authority_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CA_CERT_DER),
            [0_u8, 1_u8],
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.ca_cert_der, [0_u8, 1_u8]);
        assert_ne!(second.ca_cert_der, first.ca_cert_der);
        assert!(CertificatePaths::all_exist(directory.path()));
        Ok(())
    }

    #[test]
    fn regenerates_a_corrupt_client_key_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CLIENT_KEY_DER),
            [0_u8, 1_u8],
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.client_key_der, [0_u8, 1_u8]);
        assert_ne!(second.client_key_der, first.client_key_der);
        Ok(())
    }

    #[test]
    fn regenerates_a_mismatched_authority_cache() -> TestResult {
        let directory = tempfile::tempdir()?;
        let first = ensure(directory.path())?;
        let foreign =
            CertificateBundle::generate_at("app.tokamak.local", OffsetDateTime::now_utc())?;
        std::fs::write(
            directory.path().join(CertificatePaths::CA_CERT_DER),
            &foreign.ca_cert_der,
        )?;

        let second = ensure(directory.path())?;

        assert_ne!(second.ca_cert_der, foreign.ca_cert_der);
        assert_ne!(second.ca_cert_der, first.ca_cert_der);
        Ok(())
    }

    #[test]
    fn exposes_the_client_certificate_in_der() -> TestResult {
        let bundle =
            CertificateBundle::generate_at("app.tokamak.local", OffsetDateTime::now_utc())?;

        let der = bundle
            .client_certificate_der()
            .ok_or_else(|| std::io::Error::other("client certificate should decode"))?;

        assert!(!der.is_empty());
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

        assert!(bundle.ca_cert_pem.starts_with(CERTIFICATE));
        assert!(bundle.server_cert_pem.starts_with(CERTIFICATE));
        assert!(bundle.server_key_pem.starts_with(PRIVATE_KEY));
        assert!(bundle.ca_key_pem.starts_with(PRIVATE_KEY));
        assert!(bundle.server_identity_pem.contains(&bundle.server_cert_pem));
        assert!(bundle.server_identity_pem.contains(&bundle.server_key_pem));
        assert!(bundle.client_cert_pem.starts_with(CERTIFICATE));
        assert!(bundle.client_key_pem.starts_with(PRIVATE_KEY));
        assert!(!bundle.client_key_der.is_empty());
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

const RETRY_DELAY: StdDuration = StdDuration::from_hours(1);

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
        let bundle = CertificateBundle::ensure(&state_dir, &host, OffsetDateTime::now_utc())?;
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
            bundle.server_cert_pem.as_bytes(),
            bundle.server_key_pem.as_bytes(),
            bundle.ca_cert_pem.as_bytes(),
        )?;
        *cached = Some(Arc::clone(&config));
        Ok(config)
    }

    pub(crate) fn start_renewal(self: &Arc<Self>, events: Events) -> Renewal {
        Renewal::start(Arc::clone(self), events)
    }

    /// Renew any certificate that is due.
    pub(crate) fn refresh(&self) -> Result<()> {
        let bundle =
            CertificateBundle::ensure(&self.state_dir, &self.host, OffsetDateTime::now_utc())?;
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

    fn next_delay(&self) -> StdDuration {
        self.current.read().map_or(RETRY_DELAY, |held| {
            held.renewal_delay(OffsetDateTime::now_utc())
        })
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
            Challenge::ServerTrust { .. } => Decision::TrustAuthority(current.ca_cert_der.clone()),
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
        certificate_der(certificate) == certificate_der(&current.server_cert_pem)
    }
}

fn client_identity(current: &CertificateBundle, previous_failures: usize) -> Decision {
    if previous_failures > 0 {
        return Decision::Cancel;
    }
    current
        .client_certificate_der()
        .map_or(Decision::Cancel, |certificate| Decision::PresentIdentity {
            certificate,
            private_key: current.client_key_der.clone(),
        })
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
