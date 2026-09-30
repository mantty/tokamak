//! Certificate generation and atomic cache storage helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use der::asn1::{Ia5String, OctetString, Utf8StringRef};
use der::pem::LineEnding;
use der::{Any, Decode, Encode};
use p256::ecdsa::{DerSignature, SigningKey};
use p256::pkcs8::EncodePrivateKey;
use sha2::{Digest, Sha256};
use spki::{SubjectPublicKeyInfoOwned, SubjectPublicKeyInfoRef};
use x509_cert::attr::AttributeTypeAndValue;
use x509_cert::builder::profile::BuilderProfile;
use x509_cert::builder::{Builder, CertificateBuilder, Error as BuildError};
use x509_cert::certificate::TbsCertificate;
use x509_cert::ext::Extension;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{
    BasicConstraints, ExtendedKeyUsage, KeyUsage, KeyUsages, SubjectAltName, SubjectKeyIdentifier,
};
use x509_cert::name::{Name, RdnSequence, RelativeDistinguishedName};
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::{Time, Validity};
use x509_cert::{Certificate, der};

use crate::Result;

/// The result of building or encoding certificate material.
pub(super) type BuildResult<T> = std::result::Result<T, BuildError>;

const DAY: Duration = Duration::from_hours(24);

/// The serial number, validity and basic constraint of a kind of certificate.
struct Kind {
    serial: u64,
    validity_days: u32,
    ca: bool,
}

const AUTHORITY: Kind = Kind {
    serial: 1,
    validity_days: 3_650,
    ca: true,
};
const SERVER: Kind = Kind {
    serial: 2,
    validity_days: 90,
    ca: false,
};
const CLIENT: Kind = Kind {
    serial: 3,
    validity_days: 90,
    ca: false,
};
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A certificate's issuer: its subject name and signing key.
pub(super) struct Issuer<'a> {
    pub(super) name: Name,
    pub(super) key: &'a SigningKey,
}

/// The subject and issuer of a certificate. Its extensions are added to the
/// builder instead.
struct Names {
    subject: Name,
    issuer: Name,
}

impl BuilderProfile for Names {
    fn get_issuer(&self, _subject: &Name) -> Name {
        self.issuer.clone()
    }

    fn get_subject(&self) -> Name {
        self.subject.clone()
    }

    fn build_extensions(
        &self,
        _spk: SubjectPublicKeyInfoRef<'_>,
        _issuer_spk: SubjectPublicKeyInfoRef<'_>,
        _tbs: &TbsCertificate,
    ) -> x509_cert::builder::Result<Vec<Extension>> {
        Ok(Vec::new())
    }
}

/// A self-signed CA certificate for `key`.
pub(super) fn build_ca_certificate(key: &SigningKey, now: SystemTime) -> BuildResult<Certificate> {
    let name = common_name("tokamak local ca")?;
    let issuer = Issuer {
        name: name.clone(),
        key,
    };
    let mut builder = certificate_builder(name, &issuer, key, &AUTHORITY, now)?;
    builder.add_extension(&KeyUsage(KeyUsages::KeyCertSign | KeyUsages::CRLSign))?;
    builder.build::<_, DerSignature>(key)
}

/// A certificate for the gateway serving `host`, issued by `issuer`.
pub(super) fn build_server_certificate(
    key: &SigningKey,
    issuer: &Issuer<'_>,
    host: &str,
    now: SystemTime,
) -> BuildResult<Certificate> {
    let subject = common_name(host)?;
    let mut builder = certificate_builder(subject, issuer, key, &SERVER, now)?;
    builder.add_extension(&SubjectAltName(vec![
        GeneralName::DnsName(Ia5String::new(host)?),
        GeneralName::DnsName(Ia5String::new("localhost")?),
        GeneralName::IpAddress(OctetString::new([127, 0, 0, 1])?),
    ]))?;
    builder.add_extension(&ExtendedKeyUsage(vec![
        const_oid::db::rfc5280::ID_KP_SERVER_AUTH,
    ]))?;
    builder.build::<_, DerSignature>(issuer.key)
}

/// A client-authentication certificate issued by `issuer`.
pub(super) fn build_client_certificate(
    key: &SigningKey,
    issuer: &Issuer<'_>,
    now: SystemTime,
) -> BuildResult<Certificate> {
    let subject = common_name("tokamak client")?;
    let mut builder = certificate_builder(subject, issuer, key, &CLIENT, now)?;
    builder.add_extension(&ExtendedKeyUsage(vec![
        const_oid::db::rfc5280::ID_KP_CLIENT_AUTH,
    ]))?;
    builder.build::<_, DerSignature>(issuer.key)
}

/// The distinguished name holding only the common name `value`, as a
/// `UTF8String`. A `Name` is only built by decoding one.
fn common_name(value: &str) -> der::Result<Name> {
    let attribute = AttributeTypeAndValue {
        oid: const_oid::db::rfc4519::CN,
        value: Any::encode_from(&Utf8StringRef::new(value)?)?,
    };
    let mut names = RdnSequence::default();
    names.push(RelativeDistinguishedName::try_from(vec![attribute])?);
    Name::from_der(&names.to_der()?)
}

/// A new ECDSA P-256 signing key.
pub(super) fn generate_key() -> SigningKey {
    use p256::elliptic_curve::Generate;
    SigningKey::from(p256::SecretKey::generate())
}

/// `key` as PKCS#8, in PEM and DER.
pub(super) fn encode_key(key: &SigningKey) -> p256::pkcs8::Result<(String, Vec<u8>)> {
    let der = key.to_pkcs8_der()?;
    let pem = der.to_pem("PRIVATE KEY", LineEnding::LF)?;
    Ok((pem.to_string(), der.as_bytes().to_vec()))
}

/// `certificate` in PEM.
pub(super) fn certificate_pem(certificate: &Certificate) -> der::Result<String> {
    use der::EncodePem;
    certificate.to_pem(LineEnding::LF)
}

/// A builder for a certificate of `key`, with its validity, key identifier
/// and basic constraints.
fn certificate_builder(
    subject: Name,
    issuer: &Issuer<'_>,
    key: &SigningKey,
    kind: &Kind,
    now: SystemTime,
) -> BuildResult<CertificateBuilder<Names>> {
    let validity = Validity::new(
        Time::try_from(now - DAY)?,
        Time::try_from(now + DAY * kind.validity_days)?,
    );
    let public_key = SubjectPublicKeyInfoOwned::from_key(key.verifying_key())?;
    let names = Names {
        subject,
        issuer: issuer.name.clone(),
    };
    let mut builder =
        CertificateBuilder::new(names, SerialNumber::from(kind.serial), validity, public_key)?;
    // RFC 7093 method 1: the leftmost 160 bits of the SHA-256 of the key.
    let point = key.verifying_key().to_sec1_point(false);
    let identifier = OctetString::new(&Sha256::digest(point.as_bytes())[..20])?;
    builder.add_extension(&SubjectKeyIdentifier(identifier))?;
    builder.add_extension(&BasicConstraints {
        ca: kind.ca,
        path_len_constraint: None,
    })?;
    Ok(builder)
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
