#![deny(missing_docs)]

//! The tokamak runtime.
//!
//! A per-platform shell owns the application and drives this library: it
//! starts a [`Runtime`] for a packaged app, answers platform TLS challenges
//! with [`Certificates`], and reports runtime events.

#[cfg(all(feature = "native", target_os = "android"))]
mod android_jni;
#[cfg(all(feature = "native", target_vendor = "apple"))]
mod apple_ffi;
#[cfg(feature = "native")]
mod cert_generation;
#[cfg(feature = "native")]
mod cert_validation;
#[cfg(feature = "native")]
mod certificates;
mod compiler;
#[cfg(feature = "native")]
mod dev_proxy;
#[cfg(feature = "native")]
mod dispatcher;
#[cfg(feature = "native")]
mod event_loop;
#[cfg(feature = "native")]
mod fs;
#[cfg(feature = "native")]
mod gateway;
#[cfg(feature = "native")]
mod globals;
#[cfg(feature = "native")]
mod lifecycle_events;
#[cfg(feature = "native")]
mod linked;
#[cfg(feature = "native")]
mod network;
mod packaging;
mod quickjs;
#[cfg(all(test, feature = "native"))]
mod runtime_contract_tests;
mod runtime_modules;
#[cfg(feature = "native")]
mod server;
#[cfg(feature = "native")]
mod storage;
#[cfg(all(test, feature = "native"))]
mod tests;
#[cfg(feature = "native")]
pub use dev_proxy::DevProxyConfig;
#[cfg(feature = "native")]
pub use server::{Config, DevelopmentConfig, Runtime};
#[cfg(feature = "native")]
mod compat;
mod env_vars;
#[cfg(feature = "native")]
mod readiness;
#[cfg(feature = "native")]
mod tls;
mod tokamak_config;
#[cfg(feature = "native")]
mod transport;

use thiserror::Error as Fail;

#[cfg(feature = "native")]
pub use certificates::{Certificates, Challenge, Decision};
pub use env_vars::{
    Error as WorkerEnvironmentError, StorageBinding, WorkerEnvironment,
    load as load_worker_environment, store_id_problem, write as write_worker_environment,
};
#[cfg(feature = "native")]
pub use lifecycle_events::Event;
pub use packaging::{
    Error as BundleError, ModuleType, PackageLayout, WorkerManifest, compress_worker_bundle,
    compress_worker_module, decompress_worker_bundle, decompress_worker_module,
    read_worker_manifest, write_worker_manifest,
};
pub use quickjs::Error as QuickJsError;
pub use quickjs::{compile_module, compile_worker};
pub use tokamak_config::{
    Error as TokamakConfigError, PlatformValues, SHARED_PLATFORM_KEYS, TokamakConfig,
    app_name_problem, is_valid_key, parse_config as parse_tokamak_config, slug, value_problem,
};

/// The symbol the runtime's storage part exports its entry point as. An app
/// links the part only when its executable exports this symbol.
pub const STORAGE_ENTRY_POINT: &str = "tokamak_storage";

/// Runtime result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Runtime failures.
#[derive(Debug, Fail)]
pub enum Error {
    /// Operating-system IO failed.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Certificate generation failed.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Certificate(#[from] x509_cert::builder::Error),
    /// A certificate did not encode or decode.
    #[cfg(feature = "native")]
    #[error(transparent)]
    CertificateEncoding(#[from] x509_cert::der::Error),
    /// A certificate's private key did not encode or decode.
    #[cfg(feature = "native")]
    #[error(transparent)]
    CertificateKey(#[from] p256::pkcs8::Error),
    /// The JavaScript runtime failed to start or change state.
    #[cfg(feature = "native")]
    #[error(transparent)]
    QuickJs(#[from] QuickJsError),
    /// The packaged app contents are invalid.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Bundle(#[from] BundleError),
    /// The packaged Worker environment is invalid.
    #[cfg(feature = "native")]
    #[error(transparent)]
    Environment(#[from] WorkerEnvironmentError),
    /// Certificate state is held by a thread that stopped unexpectedly.
    #[cfg(feature = "native")]
    #[error("certificate state is unavailable")]
    CertificatesUnavailable,
}

/// Return the URL a shell's `WebView` loads.
#[must_use]
pub fn frontend_url(host: &str) -> String {
    format!("https://{host}/")
}

#[cfg(test)]
#[test]
fn frontend_url_uses_the_stable_host() {
    assert_eq!(
        frontend_url("app.tokamak.local"),
        "https://app.tokamak.local/"
    );
}
