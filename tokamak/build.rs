use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

#[path = "src/compiler.rs"]
mod compiler;

/// The script that removes the types from TypeScript modules.
const STRIP_TYPES: &str = "../scripts/strip-types.mts";

/// The directory of the runtime modules shared with `@tokamakdev/plugin`.
const PLUGIN_SOURCE: &str = "../plugins/core/src";

/// The runtime modules shared with `@tokamakdev/plugin`, under `PLUGIN_SOURCE`.
const PLUGIN_MODULES: [&str; 4] = [
    "rpc/values.ts",
    "rpc/streams.ts",
    "rpc/http.ts",
    "native-protocol.ts",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/compiler.rs");
    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    if env::var_os("CARGO_FEATURE_NATIVE").is_some() {
        compile_builtins()?;
        export_parts_from_tests(&target_os);
    }
    if target_os == "android" {
        println!("cargo:rustc-link-lib=log");
    }
    Ok(())
}

/// Compile every module under `src`: those under `storage/` into the storage
/// part, the others into the runtime.
fn compile_builtins() -> Result<(), Box<dyn std::error::Error>> {
    // A module added anywhere under `src` is compiled too.
    println!("cargo:rerun-if-changed=src");
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let javascript = output.join("javascript");
    let modules = builtin_sources()?;
    let plugin_modules = PLUGIN_MODULES.map(String::from);
    write_javascript(Path::new("src"), &javascript, &modules)?;
    write_javascript(Path::new(PLUGIN_SOURCE), &javascript, &plugin_modules)?;
    let (storage, mut builtins): (Vec<_>, Vec<_>) = modules
        .into_iter()
        .partition(|module| module.starts_with("storage/"));
    builtins.extend(plugin_modules);
    write_builtin_table(&output, &javascript, "builtins.rs", "BUILTINS", &builtins)?;
    write_builtin_table(
        &output,
        &javascript,
        "storage_builtins.rs",
        "STORAGE_BUILTINS",
        &storage,
    )
}

/// The path under `src` of each module there: TypeScript, and the vendored
/// JavaScript generated from npm packages.
fn builtin_sources() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut modules = Vec::new();
    for entry in WalkDir::new("src").sort_by_file_name() {
        let path = entry?.into_path();
        let module = path
            .strip_prefix("src")?
            .to_string_lossy()
            .replace('\\', "/");
        if matches!(extension(&module), Some("ts" | "js")) && !module.ends_with(".d.ts") {
            modules.push(module);
        }
    }
    Ok(modules)
}

fn extension(module: &str) -> Option<&str> {
    Path::new(module).extension()?.to_str()
}

/// Write each of `modules` under `from` to the same path under `to` as
/// JavaScript: TypeScript with its types removed, other modules unchanged.
fn write_javascript(
    from: &Path,
    to: &Path,
    modules: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut typescript = Vec::new();
    for module in modules {
        let source = from.join(module);
        println!("cargo:rerun-if-changed={}", source.display());
        if extension(module) == Some("ts") {
            typescript.push(module);
        } else {
            let destination = to.join(module);
            fs::create_dir_all(destination.parent().ok_or("module has no directory")?)?;
            fs::copy(&source, &destination)?;
        }
    }
    if typescript.is_empty() {
        return Ok(());
    }
    println!("cargo:rerun-if-changed={STRIP_TYPES}");
    let requirement = "building the runtime needs Node 22.18 or newer on PATH";
    let status = Command::new("node")
        .arg("--no-warnings=ExperimentalWarning")
        .arg(STRIP_TYPES)
        .arg(from)
        .arg(to)
        .args(typescript)
        .status()
        .map_err(|error| format!("{requirement}: {error}"))?;
    if !status.success() {
        return Err(format!("{STRIP_TYPES} failed ({requirement})").into());
    }
    Ok(())
}

/// Export the entry points of the storage and development parts from test
/// executables.
fn export_parts_from_tests(target_os: &str) {
    for entry_point in ["tokamak_storage", "tokamak_development"] {
        let argument = match target_os {
            "macos" | "ios" => format!("-Wl,-exported_symbol,_{entry_point}"),
            "windows" => format!("/EXPORT:{entry_point}"),
            _ => format!("-Wl,--export-dynamic-symbol={entry_point}"),
        };
        println!("cargo:rustc-link-arg-tests={argument}");
    }
}

/// Compile the JavaScript of `sources`, under `javascript`, to bytecode in
/// `output`, and write `table`, a Rust constant `constant` of each module's
/// name and bytecode.
fn write_builtin_table(
    output: &Path,
    javascript: &Path,
    table: &str,
    constant: &str,
    sources: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut modules = format!("const {constant}: &[(&str, &[u8])] = &[\n");
    for module in sources {
        let name = format!("tokamak:{module}");
        let source = fs::read(javascript.join(module))?;
        let bytecode = compiler::compile_module(&name, &source, compiler::SourceText::Stripped)
            .map_err(|error| format!("failed to compile {name}: {error}"))?;
        let destination = output.join(format!("{module}.qjs"));
        fs::create_dir_all(destination.parent().ok_or("builtin has no directory")?)?;
        fs::write(&destination, bytecode)?;
        writeln!(
            modules,
            "({name:?}, include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{module}.qjs\"))),"
        )?;
    }
    modules.push_str("];\n");
    fs::write(output.join(table), modules)?;
    Ok(())
}
