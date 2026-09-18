//! Certificate generation and atomic cache storage helpers.

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType, SerialNumber,
    SigningKey,
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use time::{Duration, OffsetDateTime};

use crate::Result;

const CA_VALIDITY_DAYS: i64 = 3_650;
const LEAF_VALIDITY_DAYS: i64 = 90;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn build_ca_certificate(
    key: &KeyPair,
    now: OffsetDateTime,
) -> Result<(CertificateParams, Certificate)> {
    let mut params = base_certificate_params("tokamak local ca", 1, now, CA_VALIDITY_DAYS);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let certificate = params.self_signed(key)?;
    Ok((params, certificate))
}

pub(super) fn build_server_certificate<S: SigningKey>(
    key: &KeyPair,
    ca_issuer: &Issuer<'_, S>,
    host: &str,
    now: OffsetDateTime,
) -> Result<Certificate> {
    let mut params = base_certificate_params(host, 2, now, LEAF_VALIDITY_DAYS);
    params.subject_alt_names = vec![
        SanType::DnsName(host.try_into()?),
        SanType::DnsName("localhost".try_into()?),
        SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)),
    ];
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    Ok(params.signed_by(key, ca_issuer)?)
}

pub(super) fn build_client_certificate<S: SigningKey>(
    key: &KeyPair,
    ca_issuer: &Issuer<'_, S>,
    now: OffsetDateTime,
) -> Result<Certificate> {
    let mut params = base_certificate_params("tokamak client", 3, now, LEAF_VALIDITY_DAYS);
    params.is_ca = IsCa::ExplicitNoCa;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    Ok(params.signed_by(key, ca_issuer)?)
}

pub(super) fn base_certificate_params(
    common_name: &str,
    serial: u64,
    now: OffsetDateTime,
    validity_days: i64,
) -> CertificateParams {
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, common_name);
    let mut params = CertificateParams::default();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(validity_days);
    params.serial_number = Some(SerialNumber::from(serial));
    params.distinguished_name = distinguished_name;
    params
}

pub(super) fn server_identity(certificate: &str, key: &str) -> String {
    format!("{certificate}{key}")
}

/// Canonical certificate filenames used in each packaged app work directory.
pub(crate) struct CertificatePaths;

impl CertificatePaths {
    /// CA certificate PEM filename.
    pub const CA_CERT_PEM: &'static str = "ca.cert.pem";
    /// CA private key PEM filename.
    pub const CA_KEY_PEM: &'static str = "ca.key.pem";
    /// CA certificate DER filename.
    pub const CA_CERT_DER: &'static str = "ca.cert.der";
    /// Server certificate PEM filename.
    pub const SERVER_CERT_PEM: &'static str = "server.cert.pem";
    /// Server private key PEM filename.
    pub const SERVER_KEY_PEM: &'static str = "server.key.pem";
    /// Server certificate and key PEM filename.
    pub const SERVER_IDENTITY_PEM: &'static str = "server.identity.pem";
    /// Client certificate PEM filename.
    pub const CLIENT_CERT_PEM: &'static str = "client.cert.pem";
    /// Client private key PEM filename.
    pub const CLIENT_KEY_PEM: &'static str = "client.key.pem";
    /// Client private key PKCS#8 DER filename.
    pub const CLIENT_KEY_DER: &'static str = "client.key.der";
    /// Marker written after every certificate file has been committed.
    pub const CACHE_MARKER: &'static str = ".complete";

    const ALL: &'static [&'static str] = &[
        Self::CA_CERT_PEM,
        Self::CA_KEY_PEM,
        Self::CA_CERT_DER,
        Self::SERVER_CERT_PEM,
        Self::SERVER_KEY_PEM,
        Self::SERVER_IDENTITY_PEM,
        Self::CLIENT_CERT_PEM,
        Self::CLIENT_KEY_PEM,
        Self::CLIENT_KEY_DER,
    ];

    /// Return true when all expected certificate files exist.
    #[must_use]
    pub fn all_exist(work_dir: impl AsRef<Path>) -> bool {
        let work_dir = work_dir.as_ref();
        work_dir.join(Self::CACHE_MARKER).is_file()
            && Self::ALL.iter().all(|name| {
                work_dir
                    .join(name)
                    .metadata()
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
            })
    }
}

pub(crate) fn is_private_key(name: &str) -> bool {
    matches!(
        name,
        CertificatePaths::SERVER_KEY_PEM
            | CertificatePaths::CA_KEY_PEM
            | CertificatePaths::SERVER_IDENTITY_PEM
            | CertificatePaths::CLIENT_KEY_PEM
            | CertificatePaths::CLIENT_KEY_DER
    )
}

pub(crate) fn remove_if_exists(path: impl AsRef<Path>) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn write_atomic(
    directory: &Path,
    name: &str,
    content: &[u8],
    private: bool,
) -> Result<()> {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".{name}.{}.{counter}.tmp", std::process::id()));
    let result = write_temporary(&temporary, content, private)
        .and_then(|()| fs::rename(&temporary, directory.join(name)).map_err(Into::into));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn write_temporary(path: &Path, content: &[u8], private: bool) -> Result<()> {
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
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn set_directory_permissions(directory: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
