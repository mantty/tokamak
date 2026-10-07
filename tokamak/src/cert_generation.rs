//! Certificate generation.

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

/// `key` as PKCS#8 PEM.
pub(super) fn key_pem(key: &SigningKey) -> p256::pkcs8::Result<String> {
    Ok(key.to_pkcs8_pem(LineEnding::LF)?.to_string())
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
