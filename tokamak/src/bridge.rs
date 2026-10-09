//! Runtime startup shared by the Apple and Android bridges, which receive
//! every value as text.

use std::path::PathBuf;

use crate::packaging::PackageLayout;
use crate::{Config, DevProxyConfig, DevelopmentConfig, Event, Runtime};

/// Starts a runtime serving the app packaged in `packaged_dir`.
pub(crate) fn start(
    packaged_dir: &str,
    state_dir: &str,
    storage_dir: &str,
    host: &str,
    foreground: bool,
    report: fn(&Event),
) -> Result<Runtime, String> {
    let config = Config {
        app: PackageLayout::new(packaged_dir),
        state_dir: PathBuf::from(state_dir),
        storage_dir: PathBuf::from(storage_dir),
        host: host.to_owned(),
        foreground,
    };
    Runtime::start(config, move |event| report(&event)).map_err(|error| error.to_string())
}

/// Starts a runtime forwarding requests to the development server at
/// `endpoint`.
pub(crate) fn start_development(
    state_dir: &str,
    host: &str,
    endpoint: &str,
    session_token: &str,
    foreground: bool,
    report: fn(&Event),
) -> Result<Runtime, String> {
    let config = DevelopmentConfig {
        state_dir: PathBuf::from(state_dir),
        host: host.to_owned(),
        proxy: DevProxyConfig {
            endpoint: endpoint.to_owned(),
            session_token: session_token.to_owned(),
        },
        foreground,
    };
    Runtime::start_development(config, move |event| report(&event))
        .map_err(|error| error.to_string())
}
