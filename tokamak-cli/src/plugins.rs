//! Native plugin discovery and staging.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use tokamak_cli::{
    PackVariable, Platform, PlatformPackManifest, PluginKeyKind, copy_file, is_valid_key,
};

use super::packs::command_path;

const MANIFEST: &str = "tokamak-plugin.json";

/// Words a plugin ID may not be or start with: the platform namespaces, the
/// top-level keys of the `tokamak()` options, and the first words of tok's own
/// options and environment variables, since a plugin's settings are
/// `--<id>-<key>`, `TOKAMAK_<ID>_<KEY>` and `<id>.<key>`.
pub(crate) const RESERVED_WORDS: &[&str] = &[
    "android",
    "ios",
    "macos",
    "windows",
    "name",
    "identifier",
    "icon",
    "version",
    "module",
    "build",
    "project",
    "platform",
    "skip",
    "host",
    "help",
    "vite",
    "session",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Plugin {
    schema_version: u32,
    pub(crate) id: String,
    /// The build settings the plugin accepts, by key.
    #[serde(default)]
    pub(crate) settings: BTreeMap<String, PackVariable>,
    /// A Node script, in the package, that checks the settings and adds keys
    /// to the plugin's platform section.
    build: Option<String>,
    /// Each platform namespace's section, which that platform's pack reads.
    #[serde(default)]
    platforms: BTreeMap<String, Map<String, Value>>,
    #[serde(skip)]
    root: PathBuf,
}

/// The keys each plugin's build script added to its section for a platform,
/// by plugin ID.
pub(crate) type Generated = BTreeMap<String, Map<String, Value>>;

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
    reject_shared_option_names(&plugins)?;
    Ok(plugins)
}

/// Fails when a plugin's setting has an option name, and so an environment
/// variable name, that starts with another plugin's longer ID: `secure`'s
/// `storage-x` and `--secure-storage-x`, which goes to `secure-storage`.
fn reject_shared_option_names(plugins: &[Plugin]) -> Result<()> {
    let prefixes = std::iter::once(String::new())
        .chain(
            Platform::ALL
                .iter()
                .map(|platform| format!("{}-", platform.namespace())),
        )
        .collect::<Vec<_>>();
    let prefixes = prefixes.as_slice();
    let names = plugins.iter().flat_map(|plugin| {
        plugin.settings.keys().flat_map(move |key| {
            prefixes
                .iter()
                .map(move |prefix| (plugin, key, format!("{}-{prefix}{key}", plugin.id)))
        })
    });
    for (plugin, key, name) in names {
        let longer = |other: &&Plugin| {
            other.id.len() > plugin.id.len() && name.starts_with(&format!("{}-", other.id))
        };
        if let Some(other) = plugins.iter().find(longer) {
            bail!(
                "plugin '{}' setting {key} takes the option --{name}, which belongs to plugin '{}'",
                plugin.id,
                other.id
            );
        }
    }
    Ok(())
}

/// Runs each plugin's build script for `platform` with its resolved
/// `settings`, by plugin ID, and returns the keys the scripts add to the
/// plugins' sections. Plugins without a section for the platform are skipped.
pub(crate) fn generate(
    plugins: &[Plugin],
    platform: Platform,
    settings: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Generated> {
    let namespace = platform.namespace();
    let mut generated = Generated::new();
    for plugin in plugins {
        let Some(script) = &plugin.build else {
            continue;
        };
        if !plugin.platforms.contains_key(namespace) {
            continue;
        }
        let script = plugin.package_file("build script", script)?;
        let input = json!({
            "platform": namespace,
            "settings": settings.get(&plugin.id).cloned().unwrap_or_default(),
        });
        let keys = run_build_script(&script, &input).with_context(|| {
            format!("plugin '{}' build script failed for {namespace}", plugin.id)
        })?;
        generated.insert(plugin.id.clone(), keys);
    }
    Ok(generated)
}

/// The object `script` writes to stdout, given `input` as JSON on stdin.
fn run_build_script(script: &Path, input: &Value) -> Result<Map<String, Value>> {
    let mut child = Command::new("node")
        .arg(command_path(script))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("run node")?;
    let written = child
        .stdin
        .take()
        .context("open the script's stdin")?
        .write_all(input.to_string().as_bytes());
    // A script that exits without reading its input closes the pipe; its exit status says why.
    if let Err(error) = written
        && error.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(error.into());
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        if message.trim().is_empty() {
            bail!("the script failed with {}", output.status);
        }
        bail!("{}", message.trim());
    }
    serde_json::from_slice(&output.stdout).context("the script must print a JSON object")
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

/// Check each plugin's section for the pack's platform, with the keys its
/// build script `generated`: each key is one the pack declares, with a value
/// of its kind and files inside the package or, for generated keys, existing
/// files at absolute paths.
pub(crate) fn check(
    plugins: &[Plugin],
    pack: &PlatformPackManifest,
    generated: &Generated,
) -> Result<()> {
    for plugin in plugins {
        staged_files(plugin, pack, generated.get(&plugin.id))?;
    }
    Ok(())
}

/// Stage each plugin's section for the pack's platform, with the keys its
/// build script `generated`, under `destination/<id>`.
pub(crate) fn stage(
    plugins: &[Plugin],
    pack: &PlatformPackManifest,
    generated: &Generated,
    destination: &Path,
) -> Result<()> {
    for plugin in plugins {
        let Some(files) = staged_files(plugin, pack, generated.get(&plugin.id))? else {
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
/// has one, with the keys its build script `generated`, by path in the
/// plugin's directory.
fn staged_files(
    plugin: &Plugin,
    pack: &PlatformPackManifest,
    generated: Option<&Map<String, Value>>,
) -> Result<Option<Vec<(PathBuf, Staged)>>> {
    let namespace = pack.target.platform().namespace();
    let Some(section) = plugin.platforms.get(namespace) else {
        return Ok(None);
    };
    let generated = generated.into_iter().flatten();
    if let Some((key, _)) = generated
        .clone()
        .find(|(key, _)| section.contains_key(*key))
    {
        bail!(
            "plugin '{}' build script sets {namespace} key '{key}', which its manifest sets",
            plugin.id
        );
    }
    let entries = section
        .iter()
        .map(|entry| (entry, false))
        .chain(generated.map(|entry| (entry, true)));
    let mut files = Vec::new();
    for ((key, value), is_generated) in entries {
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
            files.push(staged_file(plugin, key, kind, index, value, is_generated)?);
        }
    }
    Ok(Some(files))
}

/// The file that stages `value`, item `index` of the plugin's `key` of `kind`:
/// a single value at `<key>`, and a list's at `<key>/<index>`, followed by
/// `-<file name>` for a file. A file is in the plugin's package, or at an
/// absolute path when its build script `generated` the key.
fn staged_file(
    plugin: &Plugin,
    key: &str,
    kind: PluginKeyKind,
    index: usize,
    value: String,
    generated: bool,
) -> Result<(PathBuf, Staged)> {
    let path = Path::new(key);
    let file = |value: &str| {
        if !generated {
            return plugin.package_file(key, value);
        }
        let file = Path::new(value);
        if !file.is_absolute() || !file.is_file() {
            bail!(
                "plugin '{}' build script {key} is not an absolute path to a file: {value}",
                plugin.id
            );
        }
        Ok(file.to_path_buf())
    };
    Ok(match kind {
        PluginKeyKind::String => (path.into(), Staged::Value(value)),
        PluginKeyKind::Strings => (path.join(index.to_string()), Staged::Value(value)),
        PluginKeyKind::Path => (path.into(), Staged::Copy(file(&value)?)),
        PluginKeyKind::Paths => {
            let source = file(&value)?;
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
    let starts_with =
        |name: &str, word: &str| name == word || name.starts_with(&format!("{word}-"));
    if let Some(word) = RESERVED_WORDS
        .iter()
        .find(|word| starts_with(&plugin.id, word))
    {
        bail!(
            "plugin id '{}' must not start with '{word}', which tok's own options and settings use",
            plugin.id
        );
    }
    if !plugin.settings.is_empty() && plugin.build.is_none() {
        bail!(
            "plugin '{}' declares settings without a build script to read them",
            plugin.id
        );
    }
    // `<id>.ios` holds iOS's own values, and `--<id>-ios-<key>` sets one.
    let names_a_platform = |key: &str| {
        Platform::ALL
            .iter()
            .any(|platform| starts_with(key, platform.namespace()))
    };
    if let Some(key) = plugin
        .settings
        .keys()
        .find(|key| !is_valid_key(key) || names_a_platform(key))
    {
        bail!(
            "plugin '{}' setting '{key}' must be lowercase words joined by hyphens, not starting with a platform name",
            plugin.id
        );
    }
    plugin.root = root.to_path_buf();
    Ok(plugin)
}

fn valid_plugin_id(id: &str) -> bool {
    is_valid_key(id) && id.len() <= 63
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{Generated, check, discover, generate, stage, valid_plugin_id};
    use tokamak_cli::{Platform, PlatformPackManifest, PluginKeyKind, Target};

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
        stage(
            &plugins,
            &pack(Target::IosArm64),
            &Generated::new(),
            &staged,
        )?;

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
        stage(
            &plugins,
            &pack(Target::IosArm64),
            &Generated::new(),
            &staged,
        )?;

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
            &Generated::new(),
            &staged.join("ios"),
        )?;
        stage(
            &plugins,
            &pack(Target::AndroidArm64),
            &Generated::new(),
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

            let Err(error) = check(&plugins, &pack(Target::IosArm64), &Generated::new()) else {
                return Err(format!("{section} was accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }

    #[test]
    fn rejects_reserved_ids_and_settings_named_like_platforms() -> TestResult {
        for (id, setting, build, message) in [
            (
                "build",
                "dir",
                true,
                "plugin id 'build' must not start with 'build'",
            ),
            (
                "android-auto",
                "x",
                true,
                "plugin id 'android-auto' must not start with 'android'",
            ),
            (
                "skip-project",
                "build",
                true,
                "plugin id 'skip-project' must not start with 'skip'",
            ),
            (
                "alerts",
                "ios-sound",
                true,
                "plugin 'alerts' setting 'ios-sound' must be",
            ),
            (
                "alerts",
                "android",
                true,
                "plugin 'alerts' setting 'android' must be",
            ),
            (
                "alerts",
                "Sound",
                true,
                "plugin 'alerts' setting 'Sound' must be",
            ),
            (
                "alerts",
                "sounds",
                false,
                "plugin 'alerts' declares settings without a build script",
            ),
        ] {
            let project = tempfile::tempdir()?;
            let mut manifest = serde_json::json!({
                "schemaVersion": 1,
                "id": id,
                "settings": { setting: { "kind": "path", "description": "Sounds" } },
            });
            if build {
                manifest["build"] = "build.mjs".into();
            }
            install_plugin(project.path(), &manifest)?;

            let Err(error) = discover(project.path()) else {
                return Err(format!("{id}.{setting} was accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }

    #[test]
    fn rejects_settings_whose_options_belong_to_a_plugin_with_a_longer_id() -> TestResult {
        for (short, long, message) in [
            (
                ("secure", Some("storage-x")),
                ("secure-storage", None),
                "plugin 'secure' setting storage-x takes the option --secure-storage-x, which belongs to plugin 'secure-storage'",
            ),
            (
                ("alerts", Some("sound")),
                ("alerts-ios", Some("sound")),
                "plugin 'alerts' setting sound takes the option --alerts-ios-sound, which belongs to plugin 'alerts-ios'",
            ),
        ] {
            let project = tempfile::tempdir()?;
            fs::write(
                project.path().join("package.json"),
                r#"{"dependencies":{"one":"1.0.0","two":"1.0.0"}}"#,
            )?;
            for (package, (id, setting)) in [("one", short), ("two", long)] {
                let root = project.path().join("node_modules").join(package);
                fs::create_dir_all(&root)?;
                let mut manifest = serde_json::json!({ "schemaVersion": 1, "id": id });
                if let Some(setting) = setting {
                    manifest["settings"] =
                        serde_json::json!({ setting: { "kind": "string", "description": "X" } });
                    manifest["build"] = "build.mjs".into();
                }
                fs::write(root.join("tokamak-plugin.json"), manifest.to_string())?;
            }

            let Err(error) = discover(project.path()) else {
                return Err(format!("{message}: accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }

    #[test]
    fn stages_files_the_build_script_adds_from_the_settings() -> TestResult {
        let project = tempfile::tempdir()?;
        let plugin = install_plugin(
            project.path(),
            &serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "settings": { "sounds": { "kind": "path", "description": "Sounds" } },
                "build": "build.mjs",
                "platforms": { "ios": { "class": "Alerts" }, "android": { "class": "Alerts" } },
            }),
        )?;
        fs::write(
            plugin.join("build.mjs"),
            r#"
            import { readFileSync } from "node:fs";
            const { platform, settings } = JSON.parse(readFileSync(0, "utf8"));
            if (platform === "android") {
              console.error("no sounds for android");
              process.exit(1);
            }
            console.log(JSON.stringify({ sources: [settings.sounds + "/chime.swift"] }));
            "#,
        )?;
        let sounds = project.path().join("sounds");
        fs::create_dir_all(&sounds)?;
        fs::write(sounds.join("chime.swift"), "chime")?;
        let plugins = discover(project.path())?;
        let settings = BTreeMap::from([(
            "alerts".to_owned(),
            BTreeMap::from([("sounds".to_owned(), sounds.to_string_lossy().into_owned())]),
        )]);

        let generated = generate(&plugins, Platform::Ios, &settings)?;
        let staged = project.path().join("staged");
        stage(&plugins, &pack(Target::IosArm64), &generated, &staged)?;

        assert_eq!(
            fs::read_to_string(staged.join("alerts/sources/0-chime.swift"))?,
            "chime"
        );
        let Err(error) = generate(&plugins, Platform::Android, &settings) else {
            return Err("the failing script was accepted".into());
        };
        let error = format!("{error:#}");
        assert!(
            error.contains("plugin 'alerts' build script failed for android"),
            "{error}"
        );
        assert!(error.contains("no sounds for android"), "{error}");
        Ok(())
    }

    #[test]
    fn rejects_generated_keys_the_manifest_sets_and_relative_paths() -> TestResult {
        let project = tempfile::tempdir()?;
        install_plugin(
            project.path(),
            &serde_json::json!({
                "schemaVersion": 1,
                "id": "alerts",
                "platforms": { "ios": { "class": "Alerts" } },
            }),
        )?;
        let plugins = discover(project.path())?;
        for (keys, message) in [
            (
                serde_json::json!({ "class": "Other" }),
                "build script sets ios key 'class'",
            ),
            (
                serde_json::json!({ "sources": ["chime.swift"] }),
                "build script sources is not an absolute path to a file: chime.swift",
            ),
            (
                serde_json::json!({ "sources": ["/no/such/chime.swift"] }),
                "build script sources is not an absolute path to a file: /no/such/chime.swift",
            ),
            (
                serde_json::json!({ "raw": ["/no/such/chime.wav"] }),
                "plugin 'alerts' has unknown ios key 'raw'",
            ),
        ] {
            let generated = Generated::from([(
                "alerts".to_owned(),
                keys.as_object().cloned().unwrap_or_default(),
            )]);

            let Err(error) = check(&plugins, &pack(Target::IosArm64), &generated) else {
                return Err(format!("{keys} was accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }

    #[test]
    fn reports_build_scripts_that_fail_print_no_object_or_escape_the_package() -> TestResult {
        for (script, build, message) in [
            (
                "process.exit(3);",
                "build.mjs",
                "the script failed with exit",
            ),
            (
                "console.log('[1]');",
                "build.mjs",
                "the script must print a JSON object",
            ),
            ("", "../build.mjs", "build script escapes its package"),
        ] {
            let project = tempfile::tempdir()?;
            let plugin = install_plugin(
                project.path(),
                &serde_json::json!({
                    "schemaVersion": 1,
                    "id": "alerts",
                    "build": build,
                    "platforms": { "ios": { "class": "Alerts" } },
                }),
            )?;
            fs::write(plugin.join("build.mjs"), script)?;
            fs::write(project.path().join("node_modules/build.mjs"), script)?;
            let plugins = discover(project.path())?;

            let Err(error) = generate(&plugins, Platform::Ios, &BTreeMap::new()) else {
                return Err(format!("{script} was accepted").into());
            };

            assert!(format!("{error:#}").contains(message), "{error:#}");
            assert!(generate(&plugins, Platform::Android, &BTreeMap::new())?.is_empty());
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
