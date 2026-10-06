use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

#[path = "src/compiler.rs"]
mod compiler;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/compiler.rs");
    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    if env::var_os("CARGO_FEATURE_NATIVE").is_some() {
        compile_builtins()?;
        export_storage_from_tests(&target_os);
    }
    if target_os == "android" {
        println!("cargo:rustc-link-lib=log");
    }
    Ok(())
}

/// Compile every JavaScript module under `src`: those under `storage/` into
/// the storage part, the others into the runtime.
fn compile_builtins() -> Result<(), Box<dyn std::error::Error>> {
    // A module added anywhere under `src` is compiled too.
    println!("cargo:rerun-if-changed=src");
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let (storage, builtins): (Vec<_>, Vec<_>) = builtin_sources()?
        .into_iter()
        .partition(|module| module.starts_with("storage/"));
    write_builtin_table(&output, "builtins.rs", "BUILTINS", &builtins)?;
    write_builtin_table(&output, "storage_builtins.rs", "STORAGE_BUILTINS", &storage)
}

/// The path under `src` of each JavaScript module there.
fn builtin_sources() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut modules = Vec::new();
    for entry in WalkDir::new("src").sort_by_file_name() {
        let path = entry?.into_path();
        if path.extension().is_some_and(|extension| extension == "mjs") {
            let module = path.strip_prefix("src")?.to_string_lossy();
            modules.push(module.replace('\\', "/"));
        }
    }
    Ok(modules)
}

/// Export the storage part's entry point from test executables.
fn export_storage_from_tests(target_os: &str) {
    let argument = match target_os {
        "macos" | "ios" => "-Wl,-exported_symbol,_tokamak_storage",
        "windows" => "/EXPORT:tokamak_storage",
        _ => "-Wl,--export-dynamic-symbol=tokamak_storage",
    };
    println!("cargo:rustc-link-arg-tests={argument}");
}

/// Compile `sources` to bytecode in `output`, and write `table`, a Rust
/// constant `constant` of each module's name and bytecode.
fn write_builtin_table(
    output: &Path,
    table: &str,
    constant: &str,
    sources: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut modules = format!("const {constant}: &[(&str, &[u8])] = &[\n");
    for module in sources {
        let source = Path::new("src").join(module);
        let name = format!("tokamak:{module}");
        let bytecode =
            compiler::compile_module(&name, &fs::read(&source)?, compiler::SourceText::Stripped)
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
