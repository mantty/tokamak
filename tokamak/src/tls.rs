//! TLS configuration shared by the gateway, dev proxy, sockets and fetch.

use std::sync::{Arc, LazyLock};

use rustls::crypto::CryptoProvider;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

use crate::gateway::GatewayCertificates;
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

/// Server configuration for the gateway's certificate files, optionally
/// requesting a client certificate issued by the gateway CA.
pub(crate) fn server_config(
    certificates: &GatewayCertificates,
    request_client_certificate: bool,
) -> Result<Arc<ServerConfig>, Error> {
    let chain = CertificateDer::pem_file_iter(&certificates.certificate)
        .map_err(tls_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(tls_error)?;
    let private_key = PrivateKeyDer::from_pem_file(&certificates.private_key).map_err(tls_error)?;
    let builder = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(tls_error)?;
    let builder = if request_client_certificate {
        let mut roots = RootCertStore::empty();
        for authority in CertificateDer::pem_file_iter(&certificates.ca).map_err(tls_error)? {
            roots
                .add(authority.map_err(tls_error)?)
                .map_err(tls_error)?;
        }
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            Arc::new(roots),
            provider(),
        )
        .allow_unauthenticated()
        .build()
        .map_err(tls_error)?;
        builder.with_client_cert_verifier(verifier)
    } else {
        builder.with_no_client_auth()
    };
    let config = builder
        .with_single_cert(chain, private_key)
        .map_err(tls_error)?;
    Ok(Arc::new(config))
}

pub(crate) fn tls_error(error: impl std::fmt::Display) -> Error {
    Error::Tls(error.to_string())
}
