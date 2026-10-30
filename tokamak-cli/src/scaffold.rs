//! A publishable plugin package using the public authoring API.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use tokamak_cli::Platform;

use super::plugin_api::{self, Family};
use super::plugins;

/// The description the native template's registration file gives, which `tok plugin generate`
/// writes the template's TypeScript API from.
const NATIVE_DESCRIPTION: &str = include_str!("../templates/plugin/description.json");

pub(crate) fn create(directory: &Path, platforms: &[Platform]) -> Result<()> {
    let id = plugin_id(directory)?;
    let families = families(platforms)?;
    let native = !families.is_empty();
    let class = id
        .split('-')
        .map(|word| {
            let (first, rest) = word.split_at(1);
            first.to_uppercase() + rest
        })
        .collect::<String>();
    let factory = format!("create{class}");
    let module = format!("Tokamak{class}");
    let class = format!("{module}Plugin");
    let package = android_package(id);
    let names = families
        .iter()
        .map(|(family, _)| family.name())
        .collect::<Vec<_>>();
    let imports = lines(&names, |name| {
        format!("import {{ {name} }} from \"./{name}.generated.js\";")
    });
    let namespaces = lines(&names, |name| {
        format!("import * as {name} from \"./{name}.generated.js\";")
    });
    let listeners = lines(&names, |name| format!("  {name}.onEchoed(listener);"));
    let template = |source: &str| {
        source
            .replace("{{id}}", id)
            .replace("{{factory}}", &factory)
            .replace("{{class}}", &class)
            .replace("{{module}}", &module)
            .replace("{{package}}", &package)
            .replace("{{family}}", names.first().copied().unwrap_or_default())
            .replace("{{families}}", &names.join(", "))
            .replace("{{imports}}", &imports)
            .replace("{{namespaces}}", &namespaces)
            .replace("{{listeners}}", &listeners)
            .replace(
                "{{operation}}",
                if native {
                    "echo(\"Hello\")"
                } else {
                    "getCurrentPage()"
                },
            )
    };
    // Refuse to touch an existing directory, including an empty project.
    fs::create_dir(directory)
        .with_context(|| format!("Create new plugin directory {}", directory.display()))?;
    write_typescript(directory, native, &template)?;
    let mut sections = Map::new();
    for (family, named) in &families {
        sections.extend(write_native(directory, *family, named, id, &template)?);
    }
    write_json(
        directory,
        "tokamak-plugin.json",
        &json!({
            "schemaVersion": 2, "id": id, "api": { "module": "src/index.ts", "export": factory },
            "web": { "module": "web/index.ts", "export": "web" }, "platforms": sections,
        }),
    )?;
    write_package(directory, id, families.iter().map(|(family, _)| *family))?;
    if native {
        // `tok plugin check` compares generated files byte for byte, so checkouts keep their LF.
        fs::write(
            directory.join(".gitattributes"),
            "*.generated.* text eol=lf\n",
        )?;
    }
    fs::write(
        directory.join("README.md"),
        template(include_str!("../templates/plugin/README.md")),
    )?;
    println!(
        "Created {}. Install its dependencies, then add this package to your app with npm, pnpm, or Yarn.",
        directory.display()
    );
    Ok(())
}

/// Each family with native code on `platforms`, and the platforms it runs on.
fn families(platforms: &[Platform]) -> Result<Vec<(Family, Vec<Platform>)>> {
    if let Some(platform) = platforms.iter().find(|platform| {
        !Family::ALL
            .iter()
            .any(|family| family.platforms().contains(platform))
    }) {
        bail!(
            "Plugins' native code runs on ios, macos and android, not {}",
            platform.directory_name()
        );
    }
    let families = Family::ALL.into_iter().map(|family| {
        let named = family
            .platforms()
            .iter()
            .copied()
            .filter(|platform| platforms.contains(platform))
            .collect::<Vec<_>>();
        (family, named)
    });
    Ok(families.filter(|(_, named)| !named.is_empty()).collect())
}

/// `line` for each of the family `names`, one per line.
fn lines(names: &[&str], line: impl Fn(&str) -> String) -> String {
    names
        .iter()
        .map(|name| line(name))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writes the plugin's API, its `/events` entry when it has `native` code, and its browser
/// implementation.
fn write_typescript(
    directory: &Path,
    native: bool,
    template: &dyn Fn(&str) -> String,
) -> Result<()> {
    fs::create_dir(directory.join("src"))?;
    fs::create_dir(directory.join("web"))?;
    let files = if native {
        &[
            (
                "src/index.ts",
                include_str!("../templates/plugin/native.ts"),
            ),
            (
                "src/events.ts",
                include_str!("../templates/plugin/events.ts"),
            ),
            (
                "web/index.ts",
                include_str!("../templates/plugin/native-web.ts"),
            ),
        ][..]
    } else {
        &[
            ("src/index.ts", include_str!("../templates/plugin/index.ts")),
            ("web/index.ts", include_str!("../templates/plugin/web.ts")),
        ][..]
    };
    for (path, source) in files {
        fs::write(directory.join(path), template(source))?;
    }
    Ok(())
}

/// Writes the native package of `family`, with its registration file and the files generated
/// from it, returning its class's manifest sections, by platform.
fn write_native(
    directory: &Path,
    family: Family,
    platforms: &[Platform],
    id: &str,
    template: &dyn Fn(&str) -> String,
) -> Result<Map<String, Value>> {
    let (files, class): (&[(&str, &str)], &str) = match family {
        Family::Apple => (
            &[
                (
                    "Package.swift",
                    include_str!("../templates/plugin/Package.swift"),
                ),
                (
                    "Sources/{{module}}/Plugin.swift",
                    include_str!("../templates/plugin/Plugin.swift"),
                ),
                (
                    "Sources/{{module}}/Exports.swift",
                    include_str!("../templates/plugin/Exports.swift"),
                ),
                (
                    "Sources/{{module}}/Exports.generated.swift",
                    include_str!("../templates/plugin/Exports.generated.swift"),
                ),
            ],
            "{{module}}.{{class}}",
        ),
        Family::Android => (
            &[
                (
                    "build.gradle.kts",
                    include_str!("../templates/plugin/build.gradle.kts"),
                ),
                (
                    "settings.gradle.kts",
                    include_str!("../templates/plugin/settings.gradle.kts"),
                ),
                (
                    "gradle.properties",
                    include_str!("../templates/plugin/gradle.properties"),
                ),
                (
                    "src/main/kotlin/Plugin.kt",
                    include_str!("../templates/plugin/Plugin.kt"),
                ),
                (
                    "src/main/kotlin/Exports.kt",
                    include_str!("../templates/plugin/Exports.kt"),
                ),
                (
                    "src/main/kotlin/Exports.generated.kt",
                    include_str!("../templates/plugin/Exports.generated.kt"),
                ),
            ],
            "{{package}}.{{class}}",
        ),
    };
    let native = directory.join(family.name());
    for (path, source) in files {
        let path = native.join(template(path));
        fs::create_dir_all(path.parent().unwrap_or(&native))?;
        fs::write(path, template(source))?;
    }
    fs::write(
        directory.join(family.api()),
        plugin_api::typescript(
            &serde_json::from_str(NATIVE_DESCRIPTION)?,
            id,
            family,
            platforms,
        ),
    )?;
    let class = template(class);
    Ok(platforms
        .iter()
        .map(|platform| (platform.namespace().to_owned(), json!({ "class": class })))
        .collect())
}

/// The Kotlin package and Android namespace of the plugin `id`, such as
/// `tokamak.plugins.devicetools` for `device-tools`.
fn android_package(id: &str) -> String {
    format!("tokamak.plugins.{}", id.replace('-', ""))
}

fn plugin_id(directory: &Path) -> Result<&str> {
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("The directory must name the plugin")?;
    if !plugins::valid_plugin_id(id) || plugins::reserved_prefix(id).is_some() {
        bail!(
            "Use a plugin directory name of lowercase words joined by single hyphens, without a reserved Tokamak prefix"
        );
    }
    Ok(id)
}

/// The npm package, exporting `/events` and the platform API of each of the native `families`,
/// whose scripts generate and check them.
fn write_package(
    directory: &Path,
    id: &str,
    families: impl ExactSizeIterator<Item = Family>,
) -> Result<()> {
    let mut exports = json!({ ".": "./src/index.ts" });
    let mut scripts = json!({ "check": "tsc --noEmit", "prepack": "npm run check" });
    let mut dev_dependencies = json!({ "@tokamakdev/plugin": "^0.1.0", "typescript": "^5.9.3" });
    if families.len() > 0 {
        exports["./events"] = json!("./src/events.ts");
        scripts["check"] = json!("tsc --noEmit && tok plugin check");
        dev_dependencies["@tokamakdev/tok"] = json!(format!("^{}", env!("CARGO_PKG_VERSION")));
    }
    for family in families {
        let name = family.name();
        exports[format!("./{name}")] = json!(format!("./{}", family.api()));
        scripts[format!("generate:{name}")] = json!(format!("tok plugin generate {name}"));
    }
    write_json(
        directory,
        "package.json",
        &json!({
            "name": format!("tokamak-plugin-{id}"), "version": "0.1.0", "type": "module",
            "exports": exports,
            "files": ["src", "web", "apple", "android", "tokamak-plugin.json", "README.md"],
            "scripts": scripts,
            "peerDependencies": { "@tokamakdev/plugin": "^0.1.0" },
            "devDependencies": dev_dependencies,
        }),
    )?;
    write_json(
        directory,
        "tsconfig.json",
        &json!({
            "compilerOptions": { "target": "ES2022", "module": "NodeNext", "strict": true, "noEmit": true, "lib": ["ES2022", "DOM"] },
            "include": ["src", "web"],
        }),
    )?;
    Ok(())
}

fn write_json(directory: &Path, name: &str, value: &Value) -> Result<()> {
    fs::write(
        directory.join(name),
        format!("{}\n", serde_json::to_string_pretty(value)?),
    )?;
    Ok(())
}
