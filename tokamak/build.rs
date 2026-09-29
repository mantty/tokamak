use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "src/compiler.rs"]
mod compiler;
#[path = "src/runtime_modules.rs"]
mod runtime_modules;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/compiler.rs");
    println!("cargo:rerun-if-changed=src/runtime_modules.rs");
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

fn compile_builtins() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(env::var("OUT_DIR")?);
    write_builtin_table(
        &output,
        "builtins.rs",
        "BUILTINS",
        runtime_modules::BUILTIN_SOURCES,
    )?;
    write_builtin_table(
        &output,
        "storage_builtins.rs",
        "STORAGE_BUILTINS",
        runtime_modules::STORAGE_SOURCES,
    )
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
    sources: &[&str],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut modules = format!("const {constant}: &[(&str, &[u8])] = &[\n");
    for module in sources {
        let source = Path::new("src").join(module);
        println!("cargo:rerun-if-changed={}", source.display());
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
