//! TLS configuration shared by the gateway, dev proxy, sockets and fetch.

use std::io;
use std::sync::{Arc, LazyLock};

use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer, pem::PemObject};

use crate::certificates::ServerIdentity;

static PROVIDER: LazyLock<Arc<CryptoProvider>> =
    LazyLock::new(|| Arc::new(rustls::crypto::ring::default_provider()));

static CLIENT: LazyLock<Result<Arc<ClientConfig>, rustls::Error>> =
    LazyLock::new(|| build_client_config().map(Arc::new));

pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::clone(&PROVIDER)
}

/// Client configuration verifying servers against the platform trust store.
pub(crate) fn client_config() -> Result<Arc<ClientConfig>, rustls::Error> {
    CLIENT.clone()
}

fn build_client_config() -> Result<ClientConfig, rustls::Error> {
    let builder =
        ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions()?;
    #[cfg(not(target_os = "android"))]
    let builder = {
        use rustls_platform_verifier::BuilderVerifierExt;
        builder.with_platform_verifier()?
    };
    // The platform verifier needs a JNI bootstrap on Android; bundled roots are used instead.
    #[cfg(target_os = "android")]
    let builder = builder.with_root_certificates(
        webpki_roots::TLS_SERVER_ROOTS
            .iter()
            .cloned()
            .collect::<RootCertStore>(),
    );
    Ok(builder.with_no_client_auth())
}

/// Server configuration presenting `identity`, requesting (but not requiring at
/// the TLS layer) a client certificate issued by the gateway CA.
pub(crate) fn server_config(
    identity: &ServerIdentity,
    ca_cert_pem: &[u8],
) -> io::Result<Arc<ServerConfig>> {
    let chain = identity
        .certificate_chain
        .iter()
        .map(|certificate| CertificateDer::from(certificate.clone()))
        .collect();
    let private_key = PrivatePkcs8KeyDer::from(with_public_key(&identity.private_key)).into();
    let mut roots = RootCertStore::empty();
    for authority in CertificateDer::pem_slice_iter(ca_cert_pem) {
        roots
            .add(authority.map_err(tls_error)?)
            .map_err(tls_error)?;
    }
    let verifier =
        rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider())
            .allow_unauthenticated()
            .build()
            .map_err(tls_error)?;
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(tls_error)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, private_key)
        .map_err(tls_error)?;
    Ok(Arc::new(config))
}

/// The PKCS#8 key `pkcs8` with the public key that ring requires and that a
/// P-256 key may leave out, as the JDK's do. Other keys are unchanged.
fn with_public_key(pkcs8: &[u8]) -> Vec<u8> {
    match p256::SecretKey::from_pkcs8_der(pkcs8).and_then(|key| key.to_pkcs8_der()) {
        Ok(document) => document.as_bytes().to_vec(),
        Err(_) => pkcs8.to_vec(),
    }
}

fn tls_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::with_public_key;

    #[test]
    fn leaves_keys_other_than_p256_unchanged() -> Result<(), rcgen::Error> {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384)?.serialize_der();

        assert_eq!(with_public_key(&key), key);
        Ok(())
    }
}
