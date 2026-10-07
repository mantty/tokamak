//! Native plugin discovery and staging.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};

use super::support::copy_file;
use tokamak_cli::{Platform, PlatformPackManifest, PluginKeyKind};

const MANIFEST: &str = "tokamak-plugin.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Plugin {
    schema_version: u32,
    pub(crate) id: String,
    /// Each platform namespace's section, which that platform's pack reads.
    #[serde(default)]
    platforms: BTreeMap<String, Map<String, Value>>,
    #[serde(skip)]
    root: PathBuf,
}

pub(crate) fn discover(project: &Path) -> Result<Vec<Plugin>> {
    let project = fs::canonicalize(project)?;
    let mut plugins = Vec::new();
    let mut ids = BTreeSet::new();

    for dependency in dependencies(&project)? {
        let Some(root) = package_root(&project, &dependency) else {
            continue;
        };
        let manifest = root.join(MANIFEST);
        if !manifest.is_file() {
            continue;
        }
        let plugin = load(&root, &manifest)
            .with_context(|| format!("invalid plugin package {dependency}"))?;
        if !ids.insert(plugin.id.clone()) {
            bail!("duplicate tokamak plugin id '{}'", plugin.id);
        }
        plugins.push(plugin);
    }

    plugins.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(plugins)
}

impl Plugin {
    /// `path` inside the plugin's package, which it must not escape.
    fn package_file(&self, description: &str, path: &str) -> Result<PathBuf> {
        let root = fs::canonicalize(&self.root)?;
        let resolved = fs::canonicalize(root.join(path))
            .with_context(|| format!("plugin '{}' {description} is missing: {path}", self.id))?;
        if !resolved.starts_with(&root) {
            bail!(
                "plugin '{}' {description} escapes its package: {path}",
                self.id
            );
        }
        Ok(resolved)
    }
}

/// A staged file's contents: a plugin's value, or a copy of the package file
/// it names.
enum Staged {
    Value(String),
    Copy(PathBuf),
}

/// Check each plugin's section for the pack's platform: each key is one the
/// pack declares, with a value of its kind and files inside the package.
pub(crate) fn check(plugins: &[Plugin], pack: &PlatformPackManifest) -> Result<()> {
    for plugin in plugins {
        staged_files(plugin, pack)?;
    }
    Ok(())
}

/// Stage each plugin's section for the pack's platform under
/// `destination/<id>`.
pub(crate) fn stage(
    plugins: &[Plugin],
    pack: &PlatformPackManifest,
    destination: &Path,
) -> Result<()> {
    for plugin in plugins {
        let Some(files) = staged_files(plugin, pack)? else {
            continue;
        };
        let root = destination.join(&plugin.id);
        fs::create_dir_all(&root)?;
        for (path, staged) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap_or(&root))?;
            match staged {
                Staged::Value(value) => fs::write(path, value)?,
                Staged::Copy(source) => copy_file(source, path)?,
            }
        }
    }
    Ok(())
}

/// The files that stage the plugin's section for the pack's platform, if it
/// has one, by path in the plugin's directory.
fn staged_files(
    plugin: &Plugin,
    pack: &PlatformPackManifest,
) -> Result<Option<Vec<(PathBuf, Staged)>>> {
    let namespace = pack.target.platform().namespace();
    let Some(section) = plugin.platforms.get(namespace) else {
        return Ok(None);
    };
    let mut files = Vec::new();
    for (key, value) in section {
        let Some(&kind) = pack.plugin_keys.get(key) else {
            bail!("plugin '{}' has unknown {namespace} key '{key}'", plugin.id);
        };
        let invalid = || format!("plugin '{}' has an invalid {key}", plugin.id);
        let values = match kind {
            PluginKeyKind::String | PluginKeyKind::Path => {
                vec![String::deserialize(value).with_context(invalid)?]
            }
            PluginKeyKind::Strings | PluginKeyKind::Paths => {
                Vec::deserialize(value).with_context(invalid)?
            }
        };
        for (index, value) in values.into_iter().enumerate() {
            files.push(staged_file(plugin, key, kind, index, value)?);
        }
    }
    Ok(Some(files))
}

/// The file that stages `value`, item `index` of the plugin's `key` of `kind`:
/// a single value at `<key>`, and a list's at `<key>/<index>`, followed by
/// `-<file name>` for a file.
fn staged_file(
    plugin: &Plugin,
    key: &str,
    kind: PluginKeyKind,
    index: usize,
    value: String,
) -> Result<(PathBuf, Staged)> {
    let path = Path::new(key);
    Ok(match kind {
        PluginKeyKind::String => (path.into(), Staged::Value(value)),
        PluginKeyKind::Strings => (path.join(index.to_string()), Staged::Value(value)),
        PluginKeyKind::Path => (path.into(), Staged::Copy(plugin.package_file(key, &value)?)),
        PluginKeyKind::Paths => {
            let source = plugin.package_file(key, &value)?;
            let name = source.file_name().unwrap_or_default().to_string_lossy();
            (path.join(format!("{index}-{name}")), Staged::Copy(source))
        }
    })
}

fn dependencies(project: &Path) -> Result<BTreeSet<String>> {
    let path = project.join("package.json");
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let package: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    Ok(["dependencies", "devDependencies", "peerDependencies"]
        .into_iter()
        .filter_map(|field| package.get(field)?.as_object())
        .flat_map(|dependencies| dependencies.keys().cloned())
        .collect())
}

/// The nearest `node_modules/<name>` in `project` or an ancestor, as Node resolves it.
fn package_root(project: &Path, name: &str) -> Option<PathBuf> {
    project
        .ancestors()
        .map(|directory| directory.join("node_modules").join(name))
        .find(|root| root.is_dir())
}

fn load(root: &Path, path: &Path) -> Result<Plugin> {
    let mut plugin: Plugin = serde_json::from_slice(&fs::read(path)?)?;
    if plugin.schema_version != 1 {
        bail!(
            "unsupported plugin schema version {}",
            plugin.schema_version
        );
    }
    if !valid_plugin_id(&plugin.id) {
        bail!(
            "plugin id '{}' must start with a lowercase letter and contain only lowercase letters, digits, and single hyphens",
            plugin.id
        );
    }
    let is_namespace = |name: &String| {
        Platform::ALL
            .iter()
            .any(|platform| platform.namespace() == name)
    };
    if let Some(name) = plugin.platforms.keys().find(|name| !is_namespace(name)) {
        bail!("plugin '{}' has unknown platform '{name}'", plugin.id);
    }
    plugin.root = root.to_path_buf();
    Ok(plugin)
}

fn valid_plugin_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 63 || id.starts_with('-') || id.ends_with('-') {
        return false;
    }
    id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.contains("--")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{check, discover, stage, valid_plugin_id};
    use tokamak_cli::{PlatformPackManifest, PluginKeyKind, Target};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A pack for `target` that reads a plugin's class, sources, plist,
    /// manifest and dependencies.
    fn pack(target: Target) -> PlatformPackManifest {
        PlatformPackManifest {
            tokamak_version: String::new(),
            target,
            variables: BTreeMap::new(),
            plugin_keys: BTreeMap::from([
                ("class".to_owned(), PluginKeyKind::String),
                ("sources".to_owned(), PluginKeyKind::Paths),
                ("plist".to_owned(), PluginKeyKind::Path),
                ("manifest".to_owned(), PluginKeyKind::Path),
                ("dependencies".to_owned(), PluginKeyKind::Strings),
            ]),
        }
    }

    fn write_plugin(root: &Path, id: &str) -> TestResult {
        fs::create_dir_all(root.join("ios"))?;
        fs::write(root.join("ios/Plugin.swift"), "")?;
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "platforms": { "ios": { "class": "Plugin", "sources": ["ios/Plugin.swift"] } },
        });
        fs::write(root.join("tokamak-plugin.json"), manifest.to_string())?;
        Ok(())
    }

    fn discovered_ids(project: &Path) -> TestResult<Vec<String>> {
        Ok(discover(project)?
            .into_iter()
            .map(|plugin| plugin.id)
            .collect())
    }

    #[test]
    fn discovers_plugins_from_dependencies() -> TestResult {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("package.json"),
            r#"{"dependencies":{"@tokamakdev/plugin-location":"1.0.0"}}"#,
        )?;
        let plugin = root.path().join("node_modules/@tokamakdev/plugin-location");
        fs::create_dir_all(plugin.join("ios"))?;
        fs::write(plugin.join("ios/plugin.swift"), "")?;
        fs::write(
            plugin.join("tokamak-plugin.json"),
            r#"{
  "schemaVersion": 1,
  "id": "location",
  "platforms": {
    "ios": {
      "class": "LocationPlugin",
      "sources": ["ios/plugin.swift"]
    }
  }
}"#,
        )?;

        let plugins = discover(root.path())?;
        let staged = root.path().join("staged");
        stage(&plugins, &pack(Target::IosArm64), &staged)?;

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "location");
        assert_eq!(
            fs::read_to_string(staged.join("location/class"))?,
            "LocationPlugin"
        );
        assert!(staged.join("location/sources/0-plugin.swift").is_file());
        Ok(())
    }

    #[test]
    fn discovers_plugins_from_dev_and_peer_dependencies() -> TestResult {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("package.json"),
            r#"{"devDependencies":{"camera":"1.0.0"},"peerDependencies":{"location":"1.0.0"}}"#,
        )?;
        write_plugin(&root.path().join("node_modules/camera"), "camera")?;
        write_plugin(&root.path().join("node_modules/location"), "location")?;

        assert_eq!(discovered_ids(root.path())?, ["camera", "location"]);
        Ok(())
    }

    #[test]
    fn resolves_each_dependency_to_its_nearest_node_modules_copy() -> TestResult {
        let workspace = tempfile::tempdir()?;
        let app = workspace.path().join("apps/app");
        fs::create_dir_all(&app)?;
        fs::write(
            app.join("package.json"),
            r#"{"dependencies":{"camera":"1.0.0","location":"2.0.0"}}"#,
        )?;
        write_plugin(&workspace.path().join("node_modules/camera"), "camera")?;
        write_plugin(
            &workspace.path().join("node_modules/location"),
            "hoisted-location",
        )?;
        write_plugin(&app.join("node_modules/location"), "location")?;

        assert_eq!(discovered_ids(&app)?, ["camera", "location"]);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn discovers_workspace_plugins_linked_into_an_ancestor_node_modules() -> TestResult {
        let workspace = tempfile::tempdir()?;
        let app = workspace.path().join("apps/app");
        fs::create_dir_all(&app)?;
        fs::write(
            app.join("package.json"),
            r#"{"dependencies":{"@acme/location":"workspace:*"}}"#,
        )?;
        let package = workspace.path().join("packages/location");
        write_plugin(&package, "location")?;
        fs::create_dir_all(workspace.path().join("node_modules/@acme"))?;
        std::os::unix::fs::symlink(
            &package,
            workspace.path().join("node_modules/@acme/location"),
        )?;

        let plugins = discover(&app)?;
        let staged = workspace.path().join("staged");
        stage(&plugins, &pack(Target::IosArm64), &staged)?;

        assert_eq!(plugins.len(), 1);
        assert!(staged.join("location/sources/0-Plugin.swift").is_file());
        Ok(())
    }

    #[test]
    fn rejects_platforms_that_are_not_namespaces() -> TestResult {
        for platform in ["dreamcast", "web", "ios-simulator"] {
            let root = tempfile::tempdir()?;
            install_plugin(
                root.path(),
                &serde_json::json!({
                    "schemaVersion": 1,
                    "id": "bad",
                    "platforms": { platform: { "class": "Bad" } },
                }),
            )?;

            let Err(error) = discover(root.path()) else {
                return Err(format!("platform {platform} was accepted").into());
            };

            assert!(format!("{error:#}").contains(&format!("unknown platform '{platform}'")));
        }
        Ok(())
    }

    fn install_plugin(project: &Path, manifest: &serde_json::Value) -> TestResult<PathBuf> {
        fs::write(
            project.join("package.json"),
            r#"{"dependencies":{"plugin":"1.0.0"}}"#,
        )?;
        let plugin = project.join("node_modules/plugin");
        fs::create_dir_all(&plugin)?;
        fs::write(plugin.join("tokamak-plugin.json"), manifest.to_string())?;
        Ok(plugin)
    }

    #[test]
    fn stages_each_namespace_section_without_reading_its_files() -> TestResult {
        let project = tempfile::tempdir()?;
        let plugin = install_plugin(
            project.path(),
            &serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "platforms": {
                    "ios": { "class": "Alerts", "plist": "apple/Info.plist" },
                    "android": {
                        "class": "test.Alerts",
                        "manifest": "android/AndroidManifest.xml",
                        "dependencies": ["com.example:library:1.2.3"],
                    },
                },
            }),
        )?;
        fs::create_dir_all(plugin.join("apple"))?;
        fs::create_dir_all(plugin.join("android"))?;
        fs::write(plugin.join("apple/Info.plist"), "not read")?;
        fs::write(plugin.join("android/AndroidManifest.xml"), "<manifest />")?;
        let plugins = discover(project.path())?;
        let staged = project.path().join("staged");

        stage(
            &plugins,
            &pack(Target::IosSimulatorArm64),
            &staged.join("ios"),
        )?;
        stage(
            &plugins,
            &pack(Target::AndroidArm64),
            &staged.join("android"),
        )?;

        assert_eq!(
            fs::read_to_string(staged.join("ios/alerts/class"))?,
            "Alerts"
        );
        assert_eq!(
            fs::read_to_string(staged.join("ios/alerts/plist"))?,
            "not read"
        );
        assert!(!staged.join("ios/alerts/manifest").exists());
        assert_eq!(
            fs::read_to_string(staged.join("android/alerts/manifest"))?,
            "<manifest />"
        );
        assert_eq!(
            fs::read_to_string(staged.join("android/alerts/dependencies/0"))?,
            "com.example:library:1.2.3"
        );
        Ok(())
    }

    #[test]
    fn rejects_undeclared_keys_values_of_another_kind_and_files_outside_the_package() -> TestResult
    {
        for (section, message) in [
            (
                serde_json::json!({ "class": "Alerts", "frameworks": ["UIKit"] }),
                "plugin 'alerts' has unknown ios key 'frameworks'",
            ),
            (
                serde_json::json!({ "class": "Alerts", "sources": "Alerts.swift" }),
                "plugin 'alerts' has an invalid sources",
            ),
            (
                serde_json::json!({ "class": ["Alerts"] }),
                "plugin 'alerts' has an invalid class",
            ),
            (
                serde_json::json!({ "plist": "../../package.json" }),
                "plugin 'alerts' plist escapes its package",
            ),
            (
                serde_json::json!({ "sources": ["Alerts.swift"] }),
                "plugin 'alerts' sources is missing: Alerts.swift",
            ),
        ] {
            let project = tempfile::tempdir()?;
            install_plugin(
                project.path(),
                &serde_json::json!({
                    "schemaVersion": 1,
                    "id": "alerts",
                    "platforms": { "ios": section },
                }),
            )?;
            let plugins = discover(project.path())?;

            let Err(error) = check(&plugins, &pack(Target::IosArm64)) else {
                return Err(format!("{section} was accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }

    #[test]
    fn validates_protocol_identifiers() {
        assert!(valid_plugin_id("location"));
        assert!(valid_plugin_id("photo-library2"));
        assert!(!valid_plugin_id("PhotoLibrary"));
        assert!(!valid_plugin_id("2photo"));
        assert!(!valid_plugin_id("photo--library"));
        assert!(!valid_plugin_id("../photo"));
    }
}
