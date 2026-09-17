use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "src/compiler.rs"]
mod compiler;
#[path = "src/runtime_modules.rs"]
mod runtime_modules;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/compiler.rs");
    println!("cargo:rerun-if-changed=src/runtime_modules.rs");
    if env::var_os("CARGO_FEATURE_NATIVE").is_some() {
        compile_builtins()?;
    }
    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    if target_os != "android" {
        return Ok(());
    }
    println!("cargo:rustc-link-lib=log");
    link_compiler_runtime()?;
    Ok(())
}

fn compile_builtins() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(env::var("OUT_DIR")?);
    let mut modules = String::from("const BUILTINS: &[(&str, &[u8])] = &[\n");
    for module in runtime_modules::BUILTIN_SOURCES {
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
    fs::write(output.join("builtins.rs"), modules)?;
    Ok(())
}

fn link_compiler_runtime() -> Result<(), io::Error> {
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER");
    println!("cargo:rerun-if-env-changed=CC_aarch64_linux_android");
    let compiler = env::var("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER")
        .or_else(|_| env::var("CC_aarch64_linux_android"))
        .map_err(io::Error::other)?;
    let output = Command::new(compiler)
        .arg("-print-file-name=libclang_rt.builtins-aarch64-android.a")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            "failed to locate the Android compiler runtime",
        ));
    }
    let runtime = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !Path::new(&runtime).is_file() {
        return Err(io::Error::other("Android compiler runtime is missing"));
    }
    println!("cargo:rustc-link-arg={runtime}");
    Ok(())
}
