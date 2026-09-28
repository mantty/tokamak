//! Native plugin discovery.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::support::copy_file;
use tokamak_cli::Platform;

const MANIFEST: &str = "tokamak-plugin.json";

#[derive(Clone, Debug)]
pub(crate) struct Plugin {
    pub(crate) id: String,
    root: PathBuf,
    platforms: BTreeMap<String, NativePlatform>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePlatform {
    pub(crate) class: String,
    #[serde(default)]
    sources: Vec<PathBuf>,
    #[serde(default)]
    frameworks: Vec<String>,
    /// An Info.plist whose values merge into the app's.
    plist: Option<PathBuf>,
    /// An `AndroidManifest.xml` merged below the app's manifest.
    manifest: Option<PathBuf>,
    /// Maven coordinates the plugin's Android module depends on.
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    schema_version: u32,
    id: String,
    #[serde(default)]
    platforms: BTreeMap<String, NativePlatform>,
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
    pub(crate) fn platform(&self, platform: Platform) -> Option<&NativePlatform> {
        self.platforms.get(platform.directory_name())
    }

    pub(crate) fn sources(&self, platform: Platform) -> Result<Vec<PathBuf>> {
        let Some(native) = self.platform(platform) else {
            return Ok(Vec::new());
        };
        native
            .sources
            .iter()
            .map(|source| self.package_file("source", source))
            .collect()
    }

    /// `path` inside the plugin's package, which it must not escape.
    fn package_file(&self, description: &str, path: &Path) -> Result<PathBuf> {
        let root = fs::canonicalize(&self.root)?;
        let resolved = fs::canonicalize(root.join(path)).with_context(|| {
            format!(
                "plugin '{}' {description} is missing: {}",
                self.id,
                path.display()
            )
        })?;
        if !resolved.starts_with(&root) {
            bail!(
                "plugin '{}' {description} escapes its package: {}",
                self.id,
                path.display()
            );
        }
        Ok(resolved)
    }
}

/// Stage each plugin's inputs for `platform` under `destination/<id>`: its
/// class name, numbered sources, frameworks, permissions and dependencies, and
/// its `Info.plist` and `AndroidManifest.xml` files, copied unread.
pub(crate) fn stage(plugins: &[Plugin], platform: Platform, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;

    for plugin in plugins {
        let Some(native) = plugin.platform(platform) else {
            continue;
        };
        let root = destination.join(&plugin.id);
        fs::create_dir_all(root.join("sources"))?;
        fs::write(root.join("class"), &native.class)?;

        for (index, source) in plugin.sources(platform)?.into_iter().enumerate() {
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .context("plugin source must have a UTF-8 file name")?
                .to_owned();
            copy_file(
                source,
                root.join("sources").join(format!("{index}-{file_name}")),
            )?;
        }
        write_list(&root.join("frameworks"), &native.frameworks)?;
        write_list(&root.join("permissions"), &native.permissions)?;
        write_list(&root.join("dependencies"), &native.dependencies)?;
        if let Some(plist) = &native.plist {
            copy_file(
                plugin.package_file("plist", plist)?,
                root.join("Info.plist"),
            )?;
        }
        if let Some(manifest) = &native.manifest {
            copy_file(
                plugin.package_file("manifest", manifest)?,
                root.join("AndroidManifest.xml"),
            )?;
        }
    }
    Ok(())
}

/// Write each value to its own file, named by its index.
fn write_list(directory: &Path, values: &[String]) -> Result<()> {
    fs::create_dir_all(directory)?;
    for (index, value) in values.iter().enumerate() {
        fs::write(directory.join(index.to_string()), value)?;
    }
    Ok(())
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
    let manifest: PluginManifest = serde_json::from_slice(&fs::read(path)?)?;
    if manifest.schema_version != 1 {
        bail!(
            "unsupported plugin schema version {}",
            manifest.schema_version
        );
    }
    if !valid_plugin_id(&manifest.id) {
        bail!(
            "plugin id '{}' must start with a lowercase letter and contain only lowercase letters, digits, and single hyphens",
            manifest.id
        );
    }
    for (platform_name, native) in &manifest.platforms {
        let platform = platform_name.parse::<Platform>().map_err(|_| {
            anyhow::anyhow!(
                "plugin '{}' has unknown platform '{platform_name}'",
                manifest.id
            )
        })?;
        if !platform.supports_plugins() {
            bail!(
                "plugin '{}' has unknown platform '{platform_name}'",
                manifest.id
            );
        }
        if !valid_qualified_name(&native.class)
            || (platform == Platform::Android && !native.class.contains('.'))
        {
            bail!(
                "plugin '{}' has invalid {platform_name} class '{}'",
                manifest.id,
                native.class
            );
        }
        for permission in &native.permissions {
            if !permission.contains('.') || !valid_qualified_name(permission) {
                bail!(
                    "plugin '{}' has invalid {platform_name} permission '{permission}'",
                    manifest.id
                );
            }
        }
        for dependency in &native.dependencies {
            if !valid_maven_coordinate(dependency) {
                bail!(
                    "plugin '{}' has invalid {platform_name} dependency '{dependency}'; use group:artifact:version",
                    manifest.id
                );
            }
        }
    }
    Ok(Plugin {
        id: manifest.id,
        root: root.to_path_buf(),
        platforms: manifest.platforms,
    })
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

fn valid_maven_coordinate(coordinate: &str) -> bool {
    let parts: Vec<_> = coordinate.split(':').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        })
}

fn valid_qualified_name(name: &str) -> bool {
    name.split('.').all(|part| {
        let mut bytes = part.bytes();
        bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{discover, stage, valid_maven_coordinate, valid_plugin_id, valid_qualified_name};
    use tokamak_cli::Platform;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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
      "sources": ["ios/plugin.swift"],
      "frameworks": ["CoreLocation"]
    }
  }
}"#,
        )?;

        let plugins = discover(root.path())?;

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "location");
        let platform = plugins[0]
            .platform(Platform::Ios)
            .ok_or("iOS plugin is missing")?;
        assert_eq!(platform.class, "LocationPlugin");
        assert_eq!(plugins[0].sources(Platform::Ios)?.len(), 1);
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

        assert_eq!(plugins.len(), 1);
        assert_eq!(
            plugins[0].sources(Platform::Ios)?,
            [fs::canonicalize(package.join("ios/Plugin.swift"))?]
        );
        Ok(())
    }

    #[test]
    fn rejects_unknown_platforms() -> TestResult {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("package.json"),
            r#"{"dependencies":{"plugin":"1.0.0"}}"#,
        )?;
        let plugin = root.path().join("node_modules/plugin");
        fs::create_dir_all(&plugin)?;
        fs::write(
            plugin.join("tokamak-plugin.json"),
            r#"{
  "schemaVersion": 1,
  "id": "bad",
  "platforms": {"dreamcast": {"class": "Bad"}}
}"#,
        )?;

        let Err(error) = discover(root.path()) else {
            return Err("unknown platform was accepted".into());
        };

        assert!(format!("{error:#}").contains("unknown platform 'dreamcast'"));
        Ok(())
    }

    #[test]
    fn rejects_web_as_native_platform_metadata() -> TestResult {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("package.json"),
            r#"{"dependencies":{"plugin":"1.0.0"}}"#,
        )?;
        let plugin = root.path().join("node_modules/plugin");
        fs::create_dir_all(&plugin)?;
        fs::write(
            plugin.join("tokamak-plugin.json"),
            r#"{
  "schemaVersion": 1,
  "id": "bad",
  "platforms": {"web": {"class": "Bad"}}
}"#,
        )?;

        let Err(error) = discover(root.path()) else {
            return Err("web native metadata was accepted".into());
        };

        assert!(format!("{error:#}").contains("unknown platform 'web'"));
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
    fn stages_plugin_files_without_reading_them() -> TestResult {
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

        stage(&plugins, Platform::Ios, &staged.join("ios"))?;
        stage(&plugins, Platform::Android, &staged.join("android"))?;

        assert_eq!(
            fs::read_to_string(staged.join("ios/alerts/Info.plist"))?,
            "not read"
        );
        assert!(!staged.join("ios/alerts/AndroidManifest.xml").exists());
        assert_eq!(
            fs::read_to_string(staged.join("android/alerts/AndroidManifest.xml"))?,
            "<manifest />"
        );
        assert_eq!(
            fs::read_to_string(staged.join("android/alerts/dependencies/0"))?,
            "com.example:library:1.2.3"
        );
        Ok(())
    }

    #[test]
    fn rejects_plugin_files_outside_the_package() -> TestResult {
        let project = tempfile::tempdir()?;
        install_plugin(
            project.path(),
            &serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "platforms": { "ios": { "class": "Alerts", "plist": "../../package.json" } },
            }),
        )?;
        let plugins = discover(project.path())?;

        let Err(error) = stage(&plugins, Platform::Ios, &project.path().join("staged")) else {
            return Err("a plist outside the package was staged".into());
        };

        assert!(format!("{error:#}").contains("plist escapes its package"));
        Ok(())
    }

    #[test]
    fn rejects_invalid_android_dependencies() -> TestResult {
        let project = tempfile::tempdir()?;
        install_plugin(
            project.path(),
            &serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "platforms": {
                    "android": { "class": "test.Alerts", "dependencies": ["com.example:library"] },
                },
            }),
        )?;

        let Err(error) = discover(project.path()) else {
            return Err("an invalid dependency was accepted".into());
        };

        assert!(format!("{error:#}").contains("invalid android dependency 'com.example:library'"));
        Ok(())
    }

    #[test]
    fn validates_maven_coordinates() {
        assert!(valid_maven_coordinate(
            "com.google.firebase:firebase-messaging:25.0.1"
        ));
        assert!(!valid_maven_coordinate("com.example:library"));
        assert!(!valid_maven_coordinate("com.example:library:1.0:extra"));
        assert!(!valid_maven_coordinate("com.example:library:1.0'"));
        assert!(!valid_maven_coordinate("com.example::1.0"));
    }

    #[test]
    fn validates_protocol_identifiers() {
        assert!(valid_plugin_id("location"));
        assert!(valid_plugin_id("photo-library2"));
        assert!(!valid_plugin_id("PhotoLibrary"));
        assert!(!valid_plugin_id("2photo"));
        assert!(!valid_plugin_id("photo--library"));
        assert!(!valid_plugin_id("../photo"));

        assert!(valid_qualified_name("TokamakLocationPlugin"));
        assert!(valid_qualified_name(
            "com.tokamak.plugins.location.TokamakLocationPlugin"
        ));
        assert!(!valid_qualified_name("com.tokamak.Location-Plugin"));
        assert!(!valid_qualified_name("com.tokamak.2Location"));
    }
}
