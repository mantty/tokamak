//! TLS configuration shared by the gateway, dev proxy, sockets and fetch.

use std::sync::{Arc, LazyLock};

use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

use crate::quickjs::Error;

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

/// Server configuration for the gateway's certificate PEMs, requesting (but not
/// requiring at the TLS layer) a client certificate issued by the gateway CA.
pub(crate) fn server_config(
    server_cert_pem: &[u8],
    server_key_pem: &[u8],
    ca_cert_pem: &[u8],
) -> Result<Arc<ServerConfig>, Error> {
    let chain = CertificateDer::pem_slice_iter(server_cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(tls_error)?;
    let private_key = PrivateKeyDer::from_pem_slice(server_key_pem).map_err(tls_error)?;
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

pub(crate) fn tls_error(error: impl std::fmt::Display) -> Error {
    Error::Tls(error.to_string())
}
