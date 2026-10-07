//! The tokamak Vite plugin as `tok` runs it: `tok` activates it in the
//! project's build or development command and reads what it reports.

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::paths;
use super::tokamak_config::{self, TokamakConfig};

/// How a missing report is fixed.
pub(crate) const PLUGIN_HINT: &str =
    "the Vite config must include tokamak() from @tokamakdev/tok/vite next to cloudflare()";

/// The plugin's output directory.
pub(crate) struct VitePlugin {
    output: PathBuf,
}

/// The plugin's `config.json`: the Vite root and the plugin's options.
#[derive(Deserialize, PartialEq)]
pub(crate) struct ConfigReport {
    root: PathBuf,
    config: serde_json::Value,
}

impl ConfigReport {
    /// The app's configuration.
    pub(crate) fn parse(&self) -> Result<TokamakConfig> {
        tokamak_config::parse_config(&self.root, &self.config)
    }
}

/// The plugin's `server.json`: the development server and its Worker, or why
/// the plugin cannot report the server.
#[derive(Deserialize)]
#[serde(untagged)]
enum ServerFile {
    Failed { error: String },
    Reported(ServerReport),
}

/// The development server and its Worker.
#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerReport {
    pub(crate) url: String,
    /// The PEM of the end-entity certificate of each chain an HTTPS server
    /// serves; empty when the plugin found none.
    #[serde(default)]
    pub(crate) certificates: String,
    /// The Worker's name without an environment's suffix; empty when the
    /// Wrangler configuration names none.
    #[serde(default)]
    pub(crate) worker_name: String,
}

impl VitePlugin {
    /// The plugin writing to `output`.
    pub(crate) const fn new(output: PathBuf) -> Self {
        Self { output }
    }

    /// Remove what an earlier run reported.
    pub(crate) fn clear(&self) -> Result<()> {
        paths::reset_path(&self.output)
    }

    /// The environment variable that activates the plugin.
    pub(crate) fn environment(&self) -> [(&'static str, &OsStr); 1] {
        [("TOKAMAK_VITE_OUTPUT", self.output.as_os_str())]
    }

    /// The app's configuration as `command`, which ran the plugin, reported it.
    pub(crate) fn config(&self, command: &str) -> Result<ConfigReport> {
        self.report("config.json")?.with_context(|| {
            format!("{command} did not report its tokamak configuration; {PLUGIN_HINT}")
        })
    }

    /// The development server, once the plugin has reported it.
    pub(crate) fn server(&self) -> Result<Option<ServerReport>> {
        Ok(match self.report("server.json")? {
            None => None,
            Some(ServerFile::Reported(report)) => Some(report),
            Some(ServerFile::Failed { error }) => bail!("{error}"),
        })
    }

    fn report<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let path = self.output.join(name);
        if !path.is_file() {
            return Ok(None);
        }
        serde_json::from_slice(&fs::read(&path)?)
            .map(Some)
            .with_context(|| format!("read the tokamak Vite plugin's {}", path.display()))
    }
}
