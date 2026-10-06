//! The tokamak Vite plugin as `tok` runs it: `tok` activates it in the
//! project's build or development command and reads what it reports.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::support;
use super::tokamak_config::{self, TokamakConfig};

/// How a missing report is fixed.
pub(crate) const PLUGIN_HINT: &str =
    "the Vite config must include tokamak() from @tokamakdev/tok/vite next to cloudflare()";

/// The plugin's output directory and the configuration file `tok` names.
pub(crate) struct VitePlugin {
    output: PathBuf,
    config: Option<PathBuf>,
}

/// The plugin's `config.json`: the configuration file and its `config` export.
#[derive(Deserialize)]
struct ConfigReport {
    file: Option<PathBuf>,
    config: serde_json::Value,
}

/// The plugin's `server.json`.
#[derive(Deserialize)]
struct ServerReport {
    url: String,
}

impl VitePlugin {
    /// The plugin writing to `output` and reading `config`, relative to the
    /// current directory, or its default configuration file.
    pub(crate) fn new(output: PathBuf, config: Option<&Path>) -> Result<Self> {
        let config = config.map(std::path::absolute).transpose()?;
        if let Some(config) = &config
            && !config.is_file()
        {
            bail!("tokamak configuration file not found: {}", config.display());
        }
        Ok(Self { output, config })
    }

    /// Remove what an earlier run reported.
    pub(crate) fn clear(&self) -> Result<()> {
        support::reset_path(&self.output)
    }

    /// The environment variables that activate the plugin.
    pub(crate) fn environment(&self) -> Vec<(&'static str, &OsStr)> {
        let mut environment = vec![("TOKAMAK_VITE_OUTPUT", self.output.as_os_str())];
        if let Some(config) = &self.config {
            environment.push(("TOKAMAK_CONFIG", config.as_os_str()));
        }
        environment
    }

    /// The app's configuration, once the plugin has reported it.
    pub(crate) fn config(&self) -> Result<Option<TokamakConfig>> {
        let Some(report) = self.report::<ConfigReport>("config.json")? else {
            return Ok(None);
        };
        let Some(file) = report.file else {
            return Ok(Some(TokamakConfig::default()));
        };
        Ok(Some(tokamak_config::parse_config(&file, report.config)?))
    }

    /// The development server's URL, once the plugin has reported it.
    pub(crate) fn server_url(&self) -> Result<Option<String>> {
        Ok(self
            .report::<ServerReport>("server.json")?
            .map(|report| report.url))
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
