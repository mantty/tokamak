//! A publishable plugin package using the public authoring API.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum Native {
    Apple,
    Android,
}

pub(crate) fn create(directory: &Path, native: Option<Native>) -> Result<()> {
    let id = plugin_id(directory)?;
    let class = id
        .split('-')
        .map(|word| {
            let (first, rest) = word.split_at(1);
            first.to_uppercase() + rest
        })
        .collect::<String>();
    let factory = format!("create{class}");
    let class = format!("Tokamak{class}Plugin");
    let template = |source: &str| {
        source
            .replace("{{id}}", id)
            .replace("{{factory}}", &factory)
            .replace("{{class}}", &class)
            .replace(
                "{{operation}}",
                if native.is_some() {
                    "echo(\"Hello\")"
                } else {
                    "getCurrentPage()"
                },
            )
    };
    // Refuse to touch an existing directory, including an empty project.
    fs::create_dir(directory)
        .with_context(|| format!("Create new plugin directory {}", directory.display()))?;
    fs::create_dir(directory.join("src"))?;
    let source = if native.is_some() {
        include_str!("../templates/plugin/native.ts")
    } else {
        include_str!("../templates/plugin/index.ts")
    };
    fs::write(directory.join("src/index.ts"), template(source))?;
    let platforms = match native {
        None => json!({}),
        Some(Native::Apple) => {
            fs::create_dir(directory.join("apple"))?;
            fs::write(
                directory.join("apple/Plugin.swift"),
                template(include_str!("../templates/plugin/Plugin.swift")),
            )?;
            let platform = json!({ "class": class, "sources": ["apple/Plugin.swift"] });
            json!({ "ios": platform, "macos": platform })
        }
        Some(Native::Android) => {
            fs::create_dir(directory.join("android"))?;
            fs::write(
                directory.join("android/Plugin.kt"),
                template(include_str!("../templates/plugin/Plugin.kt")),
            )?;
            json!({ "android": { "class": format!("tokamak.plugins.{class}"), "sources": ["android/Plugin.kt"] } })
        }
    };
    write_json(
        directory,
        "tokamak-plugin.json",
        &json!({
            "schemaVersion": 2, "id": id, "api": { "module": "src/index.ts", "export": factory }, "platforms": platforms,
        }),
    )?;
    write_package(directory, id)?;
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

fn plugin_id(directory: &Path) -> Result<&str> {
    let id = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("The directory must name the plugin")?;
    if !tokamak_cli::is_valid_key(id)
        || id.len() > 63
        || crate::plugins::RESERVED_WORDS
            .iter()
            .any(|word| id == *word || id.starts_with(&format!("{word}-")))
    {
        bail!(
            "Use a plugin directory name of lowercase words joined by single hyphens, without a reserved Tokamak prefix"
        );
    }
    Ok(id)
}

fn write_package(directory: &Path, id: &str) -> Result<()> {
    write_json(
        directory,
        "package.json",
        &json!({
            "name": format!("tokamak-plugin-{id}"), "version": "0.1.0", "type": "module",
            "exports": { ".": "./src/index.ts" },
            "files": ["src", "apple", "android", "tokamak-plugin.json", "README.md"],
            "peerDependencies": { "@tokamakdev/plugin": "^0.1.0" },
            "devDependencies": { "@tokamakdev/plugin": "^0.1.0", "typescript": "^5.9.3" },
            "scripts": { "check": "tsc --noEmit" },
        }),
    )?;
    write_json(
        directory,
        "tsconfig.json",
        &json!({
            "compilerOptions": { "target": "ES2022", "module": "NodeNext", "strict": true, "noEmit": true },
            "include": ["src"],
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
