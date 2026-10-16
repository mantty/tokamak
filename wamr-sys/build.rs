use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=CMakeLists.txt");
    println!("cargo:rerun-if-changed=bridge.c");
    println!("cargo:rerun-if-changed=tokamak.h");
    println!("cargo:rerun-if-changed=simde");
    println!("cargo:rerun-if-changed=vendor");
    let os = env::var("CARGO_CFG_TARGET_OS")?;
    let arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let platform = match os.as_str() {
        "macos" | "ios" => "darwin",
        "android" => "android",
        "linux" => "linux",
        "windows" => "windows",
        other => panic!("WAMR does not support target OS {other}"),
    };
    let target = match arch.as_str() {
        "aarch64" => "AARCH64",
        "x86_64" => "X86_64",
        other => panic!("WAMR does not support target architecture {other}"),
    };
    let mut config = cmake::Config::new(".");
    if os == "ios" {
        let triple = env::var("TARGET")?;
        let simulator = triple.ends_with("-sim") || arch == "x86_64";
        config.define(
            "CMAKE_OSX_SYSROOT",
            if simulator {
                "iphonesimulator"
            } else {
                "iphoneos"
            },
        );
        config.define(
            "CMAKE_OSX_ARCHITECTURES",
            if arch == "aarch64" { "arm64" } else { "x86_64" },
        );
    }
    if os == "android" {
        println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
        println!("cargo:rerun-if-env-changed=ANDROID_PLATFORM");
        let ndk = std::path::PathBuf::from(env::var("ANDROID_NDK_HOME")?);
        config.define(
            "CMAKE_TOOLCHAIN_FILE",
            ndk.join("build/cmake/android.toolchain.cmake"),
        );
        config.define("ANDROID_ABI", "arm64-v8a");
        if let Ok(platform) = env::var("ANDROID_PLATFORM") {
            config.define("ANDROID_PLATFORM", platform);
        }
    }
    let output = config
        .define("WAMR_BUILD_PLATFORM", platform)
        .define("WAMR_BUILD_TARGET", target)
        .profile("Release")
        .build();
    println!("cargo:rustc-link-search=native={}/lib", output.display());
    println!("cargo:rustc-link-lib=static=tokamak_wamr");
    if os != "windows" {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
    Ok(())
}
