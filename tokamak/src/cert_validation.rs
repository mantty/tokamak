//! Certificate parsing and validation helpers.

use std::time::SystemTime;

use der::{DecodePem, Encode};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{DerSignature, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePublicKey};
use x509_cert::Certificate;
use x509_cert::der;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::ext::pkix::name::GeneralName;

/// The contents of the first PEM block in `pem`.
pub(crate) fn pem_contents(pem: &str) -> Option<Vec<u8>> {
    der::pem::decode_vec(pem.as_bytes())
        .ok()
        .map(|(_, contents)| contents)
}

pub(crate) fn certificate_not_before(pem: &str) -> Option<SystemTime> {
    with_certificate(pem, |certificate| {
        certificate
            .tbs_certificate()
            .validity()
            .not_before
            .to_system_time()
    })
}

pub(crate) fn certificate_is_valid_now(pem: &str, now: SystemTime) -> bool {
    with_certificate(pem, |certificate| {
        let validity = certificate.tbs_certificate().validity();
        validity.not_before.to_system_time() <= now && now < validity.not_after.to_system_time()
    })
    .unwrap_or(false)
}

pub(crate) fn certificate_matches_key(certificate_pem: &str, key_pem: &str) -> bool {
    let Some(key) = p256::SecretKey::from_pkcs8_pem(key_pem)
        .ok()
        .and_then(|key| key.public_key().to_public_key_der().ok())
    else {
        return false;
    };
    with_certificate(certificate_pem, |certificate| {
        public_key_info(certificate).is_some_and(|info| info == key.as_bytes())
    })
    .unwrap_or(false)
}

pub(crate) fn certificate_is_issued_by(certificate_pem: &str, issuer_pem: &str) -> bool {
    with_certificate(certificate_pem, |certificate| {
        with_certificate(issuer_pem, |issuer| {
            certificate.tbs_certificate().issuer() == issuer.tbs_certificate().subject()
                && signature_is_valid(certificate, issuer)
        })
    })
    .flatten()
    .unwrap_or(false)
}

/// The DER `SubjectPublicKeyInfo` of `certificate`.
fn public_key_info(certificate: &Certificate) -> Option<Vec<u8>> {
    certificate
        .tbs_certificate()
        .subject_public_key_info()
        .to_der()
        .ok()
}

/// Whether `issuer`'s ECDSA P-256 key signed `certificate`.
fn signature_is_valid(certificate: &Certificate, issuer: &Certificate) -> bool {
    let Some(key) =
        public_key_info(issuer).and_then(|info| VerifyingKey::from_public_key_der(&info).ok())
    else {
        return false;
    };
    let Some(signature) = certificate
        .signature()
        .as_bytes()
        .and_then(|bytes| DerSignature::from_bytes(bytes).ok())
    else {
        return false;
    };
    let Ok(signed) = certificate.tbs_certificate().to_der() else {
        return false;
    };
    certificate.signature_algorithm().oid == const_oid::db::rfc5912::ECDSA_WITH_SHA_256
        && key.verify(&signed, &signature).is_ok()
}

/// Whether the certificate in `pem` names `host` among its DNS subject
/// alternative names.
pub(crate) fn certificate_names_host(pem: &str, host: &str) -> bool {
    with_certificate(pem, |certificate| {
        let Ok(Some((_, names))) = certificate
            .tbs_certificate()
            .get_extension::<SubjectAltName>()
        else {
            return false;
        };
        names
            .0
            .iter()
            .any(|name| matches!(name, GeneralName::DnsName(value) if value.as_str() == host))
    })
    .unwrap_or(false)
}

/// Parse `pem` and apply `inspect` to the certificate, or `None` when it does
/// not parse.
pub(crate) fn with_certificate<T>(pem: &str, inspect: impl FnOnce(&Certificate) -> T) -> Option<T> {
    Certificate::from_pem(pem.as_bytes())
        .ok()
        .map(|certificate| inspect(&certificate))
}
