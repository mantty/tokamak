use std::fs;
use std::path::Path;

use serde_json::json;
use tokamak::{
    HtmlHandling, NotFoundHandling, WranglerConfigError, WranglerModuleType, load_wrangler_config,
    load_wrangler_config_for_env, resolve_wrangler_config_path,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn load_config_error(path: &Path) -> TestResult<WranglerConfigError> {
    let Err(error) = load_wrangler_config(path) else {
        return Err(std::io::Error::other("config should fail").into());
    };
    Ok(error)
}

#[test]
fn discovers_wrangler_config_using_wrangler_order_and_parent_search() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    let nested = root.join("apps/web");
    fs::create_dir_all(&nested)?;
    fs::write(
        root.join("wrangler.toml"),
        "name = \"toml-entry\"\nmain = \"toml-entry.mjs\"",
    )?;
    fs::write(
        root.join("wrangler.jsonc"),
        r#"{ "name": "jsonc-entry", "main": "jsonc-entry.mjs" }"#,
    )?;

    let config_path = resolve_wrangler_config_path(&nested, None)?;

    assert_eq!(config_path, root.join("wrangler.jsonc"));
    Ok(())
}

#[test]
fn discovers_and_parses_json_before_jsonc_and_toml() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    let nested = root.join("apps/web");
    fs::create_dir_all(&nested)?;
    fs::write(
        root.join("wrangler.toml"),
        "name = \"toml-entry\"\nmain = \"toml-entry.mjs\"",
    )?;
    fs::write(
        root.join("wrangler.jsonc"),
        r#"{ "name": "jsonc-entry", "main": "jsonc-entry.mjs" }"#,
    )?;
    fs::write(
        root.join("wrangler.json"),
        r#"{ "name": "json-entry", "main": "json-entry.mjs", "compatibility_date": "2026-06-01" }"#,
    )?;

    let config_path = resolve_wrangler_config_path(&nested, None)?;
    let config = load_wrangler_config(&config_path)?;

    assert_eq!(config_path, root.join("wrangler.json"));
    assert_eq!(config.name, "json-entry");
    assert_eq!(config.main, root.join("json-entry.mjs"));
    Ok(())
}

#[test]
fn explicit_config_path_overrides_default_discovery() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    fs::write(
        root.join("wrangler.jsonc"),
        r#"{ "name": "ignored", "main": "ignored.mjs" }"#,
    )?;
    fs::create_dir_all(root.join("config"))?;
    let explicit = root.join("config/custom.toml");
    fs::write(&explicit, "name = \"custom\"\nmain = \"worker.mjs\"")?;

    let config_path = resolve_wrangler_config_path(root, Some(Path::new("config/custom.toml")))?;

    assert_eq!(config_path, explicit);
    Ok(())
}

#[test]
fn rejects_missing_required_wrangler_fields() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    let config_path = root.join("wrangler.jsonc");
    fs::write(&config_path, r"{}")?;

    let error = load_config_error(&config_path)?;
    assert!(matches!(
        error,
        WranglerConfigError::MissingConfigField { field: "main", .. }
    ));

    fs::write(&config_path, r#"{ "main": "worker.mjs" }"#)?;

    let error = load_config_error(&config_path)?;
    assert!(matches!(
        error,
        WranglerConfigError::MissingConfigField { field: "name", .. }
    ));

    fs::write(
        &config_path,
        r#"{ "name": "demo-app", "main": "worker.mjs", "assets": {} }"#,
    )?;

    let error = load_config_error(&config_path)?;
    assert!(matches!(
        error,
        WranglerConfigError::MissingConfigField {
            field: "assets.directory",
            ..
        }
    ));
    Ok(())
}

#[test]
fn rejects_invalid_wrangler_config_values_and_formats() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();

    let unsupported = root.join("wrangler.yaml");
    fs::write(&unsupported, "main: worker.mjs")?;
    assert!(matches!(
        load_config_error(&unsupported)?,
        WranglerConfigError::UnsupportedConfigFormat(_)
    ));

    let jsonc_path = root.join("wrangler.jsonc");
    fs::write(&jsonc_path, r#"{ "main": "#)?;
    assert!(matches!(
        load_config_error(&jsonc_path)?,
        WranglerConfigError::InvalidConfig { .. }
    ));

    fs::write(
        &jsonc_path,
        r#"{
  "name": "demo-app",
  "main": "worker.mjs",
  "assets": {
    "directory": "public",
    "html_handling": "surprising"
  }
}"#,
    )?;
    assert!(matches!(
        load_config_error(&jsonc_path)?,
        WranglerConfigError::InvalidAssetConfig(_)
    ));

    fs::write(
        &jsonc_path,
        r#"{
  "name": "demo-app",
  "main": "worker.mjs",
  "assets": {
    "directory": "public",
    "not_found_handling": "surprising"
  }
}"#,
    )?;
    assert!(matches!(
        load_config_error(&jsonc_path)?,
        WranglerConfigError::InvalidAssetConfig(_)
    ));

    fs::write(
        &jsonc_path,
        r#"{ "name": "demo-app", "main": "worker.mjs", "assets": { "directory": "public", "binding": "" } }"#,
    )?;
    assert!(matches!(
        load_config_error(&jsonc_path)?,
        WranglerConfigError::InvalidAssetConfig(_)
    ));
    Ok(())
}

#[test]
fn rejects_a_tokamak_unsafe_wrangler_name() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let config_path = temp_dir.path().join("wrangler.jsonc");
    fs::write(
        &config_path,
        r#"{ "name": "Demo_App", "main": "worker.mjs" }"#,
    )?;

    assert!(matches!(
        load_config_error(&config_path)?,
        WranglerConfigError::InvalidAppName(_)
    ));
    Ok(())
}

#[test]
fn parses_jsonc_wrangler_config_subset_and_resolves_paths() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    fs::create_dir_all(root.join("build/server"))?;
    fs::create_dir_all(root.join("build/client"))?;
    let config_path = root.join("wrangler.jsonc");
    fs::write(
        &config_path,
        r#"{
  // JSONC comments and trailing commas are valid Wrangler config.
  "name": "demo-app",
  "main": "build/server/entry.mjs",
  "compatibility_date": "2026-06-01",
  "compatibility_flags": ["nodejs_compat"],
  "vars": { "TEXT": "value", "JSON": { "enabled": true } },
  "assets": {
    "directory": "build/client",
    "binding": "STATIC",
    "html_handling": "drop-trailing-slash",
    "not_found_handling": "single-page-application",
  },
}"#,
    )?;

    let config = load_wrangler_config(&config_path)?;

    assert_eq!(config.name, "demo-app");
    assert_eq!(config.main, root.join("build/server/entry.mjs"));
    let assets = config
        .assets
        .as_ref()
        .ok_or_else(|| std::io::Error::other("assets should be parsed"))?;
    assert_eq!(assets.directory, root.join("build/client"));
    assert_eq!(assets.binding, "STATIC");
    assert_eq!(assets.html_handling, HtmlHandling::Drop);
    assert_eq!(
        assets.not_found_handling,
        NotFoundHandling::SinglePageApplication
    );
    assert_eq!(config.vars.get("TEXT"), Some(&json!("value")));
    assert_eq!(config.vars.get("JSON"), Some(&json!({ "enabled": true })));
    Ok(())
}

#[test]
fn parses_toml_wrangler_config_subset() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    let config_path = root.join("wrangler.toml");
    fs::write(
        &config_path,
        r#"
name = "demo-app"
main = "worker/entry.mjs"
compatibility_date = "2026-06-01"
compatibility_flags = ["nodejs_compat"]

[vars]
TEXT = "value"
JSON = { enabled = true }

[assets]
directory = "public"
binding = "ASSETS"
html_handling = "force-trailing-slash"
not_found_handling = "404-page"
"#,
    )?;

    let config = load_wrangler_config(&config_path)?;

    assert_eq!(config.name, "demo-app");
    assert_eq!(config.main, root.join("worker/entry.mjs"));
    let assets = config
        .assets
        .as_ref()
        .ok_or_else(|| std::io::Error::other("assets should be parsed"))?;
    assert_eq!(assets.directory, root.join("public"));
    assert_eq!(assets.html_handling, HtmlHandling::Force);
    assert_eq!(assets.not_found_handling, NotFoundHandling::Page404);
    assert_eq!(config.vars.get("TEXT"), Some(&json!("value")));
    assert_eq!(config.vars.get("JSON"), Some(&json!({ "enabled": true })));
    Ok(())
}

#[test]
fn selects_named_jsonc_vars_without_inheriting_top_level_vars() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let config = temporary.path().join("wrangler.jsonc");
    fs::write(
        &config,
        r#"{
  "name": "demo-app",
  "main": "dist/worker.mjs",
  "vars": { "API": "default", "TOP_ONLY": true },
  "env": {
    "test": { "vars": { "API": "test", "JSON": { "enabled": true } } },
    "production": { "vars": { "API": "production" } }
  }
}"#,
    )?;

    let default = load_wrangler_config_for_env(&config, None)?;
    let test = load_wrangler_config_for_env(&config, Some("test"))?;
    let production = load_wrangler_config_for_env(&config, Some("production"))?;
    assert_eq!(default.vars.get("TOP_ONLY"), Some(&json!(true)));
    assert_eq!(test.vars.get("API"), Some(&json!("test")));
    assert_eq!(test.vars.get("JSON"), Some(&json!({ "enabled": true })));
    assert!(!test.vars.contains_key("TOP_ONLY"));
    assert_eq!(production.vars.get("API"), Some(&json!("production")));
    assert_eq!(test.main, default.main);
    assert_eq!(test.name, default.name);
    assert!(matches!(
        load_wrangler_config_for_env(&config, Some("missing")),
        Err(WranglerConfigError::EnvironmentNotFound { .. })
    ));
    Ok(())
}

#[test]
fn selects_named_toml_json_vars() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let config = temporary.path().join("wrangler.toml");
    fs::write(
        &config,
        r#"
name = "demo-app"
main = "dist/worker.mjs"

[vars]
API = "default"

[env.production.vars]
API = "production"
JSON = { enabled = true, count = 3 }
"#,
    )?;

    let production = load_wrangler_config_for_env(&config, Some("production"))?;
    assert_eq!(production.vars.get("API"), Some(&json!("production")));
    assert_eq!(
        production.vars.get("JSON"),
        Some(&json!({ "enabled": true, "count": 3 }))
    );
    Ok(())
}

#[test]
fn selects_vars_from_the_source_of_a_generated_wrangler_config() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("wrangler.jsonc");
    let generated_dir = temporary.path().join("dist/server");
    fs::create_dir_all(&generated_dir)?;
    fs::write(
        &source,
        r#"{
  "name": "demo-app",
  "vars": { "API": "default" },
  "env": { "production": { "vars": { "API": "production", "JSON": { "ok": true } } } }
}"#,
    )?;
    let generated = generated_dir.join("wrangler.json");
    fs::write(
        &generated,
        serde_json::to_vec(&json!({
            "name": "demo-app",
            "main": "entry.mjs",
            "userConfigPath": source,
            "env": {
                "production": { "main": "production.mjs", "vars": { "API": "stale" } }
            }
        }))?,
    )?;

    let default = load_wrangler_config_for_env(&generated, None)?;
    let production = load_wrangler_config_for_env(&generated, Some("production"))?;
    assert_eq!(default.vars.get("API"), Some(&json!("default")));
    assert_eq!(production.vars.get("API"), Some(&json!("production")));
    assert_eq!(production.vars.get("JSON"), Some(&json!({ "ok": true })));
    assert_eq!(production.main, generated_dir.join("production.mjs"));
    Ok(())
}

#[test]
fn parses_exact_asset_paths() -> TestResult {
    assert_eq!(HtmlHandling::parse("none")?, HtmlHandling::None);
    assert_eq!(HtmlHandling::None.as_str(), "none");
    Ok(())
}

#[test]
fn parses_additional_module_rules_and_base_directory() -> TestResult {
    let temp_dir = tempfile::tempdir()?;
    let root = temp_dir.path();
    let config_path = root.join("wrangler.jsonc");
    fs::write(
        &config_path,
        r#"{
  "name": "demo-app",
  "main": "worker/entry.mjs",
  "base_dir": "worker",
  "find_additional_modules": true,
  "rules": [
    { "type": "Text", "globs": ["**/*.md"] },
    { "type": "Data", "globs": ["**/*.bin"], "fallthrough": true }
  ]
}"#,
    )?;

    let config = load_wrangler_config(&config_path)?;

    assert_eq!(config.base_dir, root.join("worker"));
    assert!(config.find_additional_modules);
    assert_eq!(config.rules.len(), 2);
    assert_eq!(config.rules[0].module_type, WranglerModuleType::Text);
    assert_eq!(config.rules[0].globs, ["**/*.md"]);
    assert!(config.rules[1].fallthrough);
    Ok(())
}
