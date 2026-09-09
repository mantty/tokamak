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
mod asset_manifest;
mod builtins;
#[cfg(feature = "native")]
mod cert_generation;
#[cfg(feature = "native")]
mod cert_validation;
#[cfg(feature = "native")]
mod certificates;
#[cfg(feature = "native")]
mod dev_proxy;
#[cfg(feature = "native")]
mod dispatcher;
mod events;
#[cfg(feature = "native")]
mod fs;
#[cfg(feature = "native")]
mod gateway;
mod globals;
#[cfg(feature = "native")]
mod lifecycle_events;
mod network;
mod packaging;
mod quickjs;
#[cfg(feature = "native")]
mod server;
#[cfg(all(test, feature = "native"))]
mod tests;
#[cfg(feature = "native")]
pub use dev_proxy::DevProxyConfig;
#[cfg(feature = "native")]
pub use server::{Config, DevelopmentConfig, Runtime};
mod compat;
mod env_vars;
mod streams;
mod tokamak_config;
#[cfg(feature = "native")]
mod transport;
mod wrangler_config;

use thiserror::Error as Fail;

pub use asset_manifest::{Error as AssetManifestError, write_manifest as write_asset_manifest};
#[cfg(feature = "native")]
pub use certificates::{Certificates, Challenge, Decision};
pub use compat::write_worker_compatibility_sources;
pub use env_vars::{
    Error as WorkerEnvironmentError, WorkerEnvironment, load as load_worker_environment,
    write as write_worker_environment,
};
#[cfg(feature = "native")]
pub use lifecycle_events::Event;
pub use packaging::{
    Error as BundleError, PackageLayout, WorkerManifest, compress_worker_bundle,
    compress_worker_module, decompress_worker_bundle, decompress_worker_module,
    read_worker_manifest, write_worker_manifest,
};
pub use quickjs::Error as QuickJsError;
pub use quickjs::{compile_module, compile_worker};
pub use tokamak_config::{
    Error as TokamakConfigError, TokamakConfig, TokamakIcons, TokamakName,
    load_config as load_tokamak_config, resolve_config_path as resolve_tokamak_config_path,
};
pub use wrangler_config::{
    Error as WranglerConfigError, HtmlHandling, NotFoundHandling, WranglerAssets, WranglerBinding,
    WranglerConfig, WranglerModuleType, WranglerRule, app_host, is_valid_app_name,
    load_config as load_wrangler_config, resolve_config_path as resolve_wrangler_config_path,
};

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
    Certificate(#[from] rcgen::Error),
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
