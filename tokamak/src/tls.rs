//! TLS configuration shared by the gateway, dev proxy, sockets and fetch.

use std::io;
use std::sync::{Arc, LazyLock};

#[cfg(target_os = "android")]
use rustls::RootCertStore;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig,
    SignatureScheme,
};
use rustls_pki_types::{CertificateDer, PrivatePkcs8KeyDer, UnixTime};
use x509_cert::Certificate;
use x509_cert::der::{DecodePem, Encode};

use crate::cert_validation::client_certificate_is_issued_by;
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

/// Server configuration presenting `identity`, requesting (but not requiring
/// at the TLS layer) a client certificate that `authority`, a PEM certificate,
/// issued.
pub(crate) fn server_config(
    identity: &ServerIdentity,
    authority: &str,
) -> io::Result<Arc<ServerConfig>> {
    let chain = identity
        .certificate_chain
        .iter()
        .map(|certificate| CertificateDer::from(certificate.clone()))
        .collect();
    let private_key = PrivatePkcs8KeyDer::from(identity.private_key.clone()).into();
    let authority = Certificate::from_pem(authority.as_bytes()).map_err(tls_error)?;
    let subject = authority
        .tbs_certificate()
        .subject()
        .to_der()
        .map_err(tls_error)?;
    let verifier = IssuedClientCerts {
        authority,
        subjects: vec![DistinguishedName::from(subject)],
        algorithms: PROVIDER.signature_verification_algorithms,
    };
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(tls_error)?
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(chain, private_key)
        .map_err(tls_error)?;
    Ok(Arc::new(config))
}

/// Accepts the client certificates the runtime's `authority` issued, at any
/// date. The handshake proves possession of the certificate's key, and the
/// runtime issues and renews the certificates itself, so the device's clock
/// does not decide validity. Certificates from before a renewal still pass:
/// a `WebView` keeps presenting the one it was first given.
#[derive(Debug)]
struct IssuedClientCerts {
    authority: Certificate,
    subjects: Vec<DistinguishedName>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for IssuedClientCerts {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.subjects
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        if client_certificate_is_issued_by(end_entity, &self.authority) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn tls_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
