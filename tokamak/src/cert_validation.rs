//! Certificate parsing and validation helpers.

use rcgen::{KeyPair, PublicKeyData};
use time::OffsetDateTime;
use x509_parser::{certificate::X509Certificate, parse_x509_certificate, pem::parse_x509_pem};

pub(crate) fn certificate_der(pem: &str) -> Option<Vec<u8>> {
    parse_x509_pem(pem.as_bytes())
        .ok()
        .map(|(_, certificate)| certificate.contents)
}

pub(crate) fn certificate_der_matches_pem(certificate_pem: &str, certificate_der: &[u8]) -> bool {
    let Ok((_, pem)) = parse_x509_pem(certificate_pem.as_bytes()) else {
        return false;
    };
    pem.contents == certificate_der && parse_x509_certificate(certificate_der).is_ok()
}

pub(crate) fn key_matches_der(key_pem: &str, key_der: &[u8]) -> bool {
    KeyPair::from_pem(key_pem).is_ok_and(|key| key.serialized_der() == key_der)
}

pub(crate) fn certificate_not_before(pem: &str) -> Option<OffsetDateTime> {
    with_certificate(pem, |certificate| {
        OffsetDateTime::from_unix_timestamp(certificate.validity().not_before.timestamp()).ok()
    })
    .flatten()
}

pub(crate) fn certificate_is_valid_now(pem: &str, now: OffsetDateTime) -> bool {
    with_certificate(pem, |certificate| {
        let now = now.unix_timestamp();
        let validity = certificate.validity();
        validity.not_before.timestamp() <= now && now < validity.not_after.timestamp()
    })
    .unwrap_or(false)
}

pub(crate) fn certificate_matches_key(certificate_pem: &str, key_pem: &str) -> bool {
    let Ok(key) = KeyPair::from_pem(key_pem) else {
        return false;
    };
    with_certificate(certificate_pem, |certificate| {
        certificate.public_key().raw == key.subject_public_key_info()
    })
    .unwrap_or(false)
}

pub(crate) fn certificate_is_issued_by(certificate_pem: &str, issuer_pem: &str) -> bool {
    with_certificate(certificate_pem, |certificate| {
        with_certificate(issuer_pem, |issuer| {
            certificate.issuer() == issuer.subject()
                && certificate
                    .verify_signature(Some(issuer.public_key()))
                    .is_ok()
        })
    })
    .flatten()
    .unwrap_or(false)
}

/// Parse `pem` and apply `inspect` to the certificate, or `None` when it does
/// not parse.
pub(crate) fn with_certificate<T>(
    pem: &str,
    inspect: impl FnOnce(&X509Certificate<'_>) -> T,
) -> Option<T> {
    let (_, pem) = parse_x509_pem(pem.as_bytes()).ok()?;
    let certificate = pem.parse_x509().ok()?;
    Some(inspect(&certificate))
}
