//! The Apple platform pack's entrypoint, run as the CLI runs it: from a pack
//! root holding a test runtime, a stub shell, the plugin API, and the real
//! Apple tool, whose signing is faked.
#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};

const TOOL: &str = env!("CARGO_BIN_EXE_tokamak-apple-signing");
const BUILD: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../build");
const SOURCE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../source");
const PLUGIN_API: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../plugins/core/apple/TokamakPlugin"
);

/// A pack for one target, an app's build input, and the log of the fake tools.
struct Fixture {
    root: tempfile::TempDir,
    target: &'static str,
}

impl Fixture {
    fn new(target: &'static str) -> Result<Self> {
        let fixture = Self {
            root: tempfile::tempdir()?,
            target,
        };
        fixture.create_pack()?;
        let metadata = fixture.input().join("metadata");
        fs::create_dir_all(&metadata)?;
        fs::create_dir_all(fixture.input().join("plugins"))?;
        for (name, value) in [
            ("app-name", "Demo App"),
            ("app-slug", "demo-app"),
            ("identifier", "com.example.demo"),
            ("host", "demo-app.tokamak.local"),
            ("platform", fixture.platform()),
            ("target", target),
            ("project-dir", path_text(fixture.root.path())?),
            ("version", "1.2.3"),
            ("exported-symbols", ""),
        ] {
            fs::write(metadata.join(name), value)?;
        }
        fs::create_dir_all(fixture.input().join("app"))?;
        fs::write(fixture.input().join("app/index.html"), "<html></html>")?;
        fs::write(fixture.input().join("app/worker-environment.json"), "{}")?;
        Ok(fixture)
    }

    fn create_pack(&self) -> Result<()> {
        let pack = self.pack();
        fs::create_dir_all(pack.join("build"))?;
        for file in [
            "entrypoint",
            "ios-deployment-target",
            "macos-deployment-target",
        ] {
            fs::copy(Path::new(BUILD).join(file), pack.join("build").join(file))?;
        }
        fs::create_dir_all(pack.join("native-shell"))?;
        fs::write(
            pack.join("native-shell/TokamakShell.swift"),
            include_str!("fixtures/TokamakShell.swift"),
        )?;
        self.copy_plugin_api()?;
        self.create_runtime()?;
        let log = self.log();
        let log = path_text(&log)?;
        let app = self.bundle_app_path();
        write_executable(
            &pack.join("tools/tokamak-apple-signing"),
            &format!(
                r#"#!/bin/sh
set -eu
if [ "$1" != sign ]; then
  exec '{TOOL}' "$@"
fi
while [ "$#" -gt 1 ]; do
  if [ "$1" = --bundle ]; then
    bundle=$2
  fi
  shift
done
settings="${{TOKAMAK_IOS_SIGNING_IDENTITY:-}}${{TOKAMAK_IOS_TEAM_ID:-}}${{TOKAMAK_MACOS_TEAM_ID:-}}"
printf 'sign %s %s\n' "$settings" "$(cat "$bundle/{app}/worker-environment.json")" >> '{log}'
"#
            ),
        )?;
        self.create_fake_tools()
    }

    /// The `codesign`, `xcrun`, and `xcodebuild` that fake builds run, which
    /// log their work.
    fn create_fake_tools(&self) -> Result<()> {
        let log = self.log();
        let log = path_text(&log)?;
        let app = self.bundle_app_path();
        write_executable(
            &self.fake_tools().join("codesign"),
            &format!(
                r#"#!/bin/sh
for bundle in "$@"; do :; done
printf 'sign - %s\n' "$(cat "$bundle/{app}/worker-environment.json")" >> '{log}'
"#
            ),
        )?;
        write_executable(
            &self.fake_tools().join("xcrun"),
            &format!(
                r#"#!/bin/sh
set -eu
case " $* " in
  *" actool "*|*" strip "*) ;;
  *) exec /usr/bin/xcrun "$@" ;;
esac
printf '%s\n' "$*" >> '{log}'
next=
for argument in "$@"; do
  case "$next" in
    compile) mkdir -p "$argument" && printf assets > "$argument/Assets.car" ;;
    partial) printf '%s' '<plist version="1.0"><dict><key>CFBundleIconName</key><string>AppIcon</string></dict></plist>' > "$argument" ;;
  esac
  case "$argument" in
    --compile) next=compile ;;
    --output-partial-info-plist) next=partial ;;
    *) next= ;;
  esac
done
"#
            ),
        )?;
        // Writes the app's executable where a build for the destination does.
        write_executable(
            &self.fake_tools().join("xcodebuild"),
            &format!(
                r#"#!/bin/sh
set -eu
if [ "$1" = -version ]; then
  exec /usr/bin/xcodebuild "$@"
fi
printf 'xcodebuild %s\n' "$*" >> '{log}'
next=
for argument in "$@"; do
  case "$next" in
    derived) derived=$argument ;;
    destination) destination=$argument ;;
  esac
  case "$argument" in
    -derivedDataPath) next=derived ;;
    -destination) next=destination ;;
    *) next= ;;
  esac
done
case "$destination" in
  *'iOS Simulator') products=Release-iphonesimulator ;;
  *iOS) products=Release-iphoneos ;;
  *) products=Release ;;
esac
mkdir -p "$derived/Build/Products/$products"
printf '#!/bin/sh\n' > "$derived/Build/Products/$products/App"
"#
            ),
        )
    }

    /// The plugin API package, which plugins' packages depend on.
    fn copy_plugin_api(&self) -> Result<()> {
        let sources = self
            .pack()
            .join("plugin-api/TokamakPlugin/Sources/TokamakPlugin");
        fs::create_dir_all(&sources)?;
        fs::copy(
            Path::new(PLUGIN_API).join("Package.swift"),
            self.pack().join("plugin-api/TokamakPlugin/Package.swift"),
        )?;
        for source in fs::read_dir(Path::new(PLUGIN_API).join("Sources/TokamakPlugin"))? {
            let source = source?;
            fs::copy(source.path(), sources.join(source.file_name()))?;
        }
        Ok(())
    }

    /// A static runtime library that defines the storage entry point, with
    /// the runtime's header, as the pack's `XCFramework`.
    fn create_runtime(&self) -> Result<()> {
        let source = self.root.path().join("runtime.c");
        let object = self.root.path().join("runtime.o");
        let library = self.root.path().join("libtokamak.a");
        let headers = self.root.path().join("headers");
        fs::write(
            &source,
            "void tokamak_test(void) {}\nconst char tokamak_storage = 0;",
        )?;
        let (sdk, triple) = compile_target(self.target)?;
        run(Command::new("xcrun")
            .args(["--sdk", sdk, "clang", "-target", &triple, "-c"])
            .arg(&source)
            .arg("-o")
            .arg(&object))?;
        run(Command::new("xcrun")
            .args(["libtool", "-static", "-o"])
            .arg(&library)
            .arg(&object))?;
        fs::create_dir_all(&headers)?;
        for header in ["TokamakRuntime.h", "module.modulemap"] {
            fs::copy(Path::new(SOURCE).join(header), headers.join(header))?;
        }
        run(Command::new("xcodebuild")
            .arg("-create-xcframework")
            .arg("-library")
            .arg(&library)
            .arg("-headers")
            .arg(&headers)
            .arg("-output")
            .arg(self.pack().join("frameworks/TokamakRuntime.xcframework")))
    }

    fn platform(&self) -> &'static str {
        if self.target.starts_with("macos") {
            "macos"
        } else {
            "ios"
        }
    }

    /// The pack variable `key`, such as `ICON`, for the fixture's platform.
    fn variable(&self, key: &str) -> String {
        format!("TOKAMAK_{}_{key}", self.platform().to_ascii_uppercase())
    }

    fn pack(&self) -> PathBuf {
        self.root.path().join("pack")
    }

    fn input(&self) -> PathBuf {
        self.root.path().join("input")
    }

    fn output(&self) -> PathBuf {
        self.root.path().join("build/demo-app.app")
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("tools.log")
    }

    fn fake_tools(&self) -> PathBuf {
        self.root.path().join("fake-tools")
    }

    /// Where the app's package lives in the bundle.
    fn bundle_app_path(&self) -> &'static str {
        if self.platform() == "macos" {
            "Contents/Resources/app"
        } else {
            "app"
        }
    }

    /// The entrypoint's build command, without the developer's tokamak settings.
    fn build(&self) -> Command {
        let mut command = Command::new("bash");
        command
            .arg("build/entrypoint")
            .arg("build")
            .arg(self.input())
            .arg(self.output())
            .current_dir(self.pack());
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("TOKAMAK_") {
                command.env_remove(name);
            }
        }
        command
    }

    /// The build command with fake `actool`, `xcodebuild`, `strip`, and `codesign`.
    fn fake_build(&self) -> Result<Command> {
        let mut path = OsString::from(self.fake_tools());
        path.push(":");
        path.push(std::env::var_os("PATH").context("PATH is set")?);
        let mut command = self.build();
        command.env("PATH", path);
        Ok(command)
    }

    fn plist(&self) -> Result<Dictionary> {
        let contents = if self.platform() == "macos" {
            "Contents/"
        } else {
            ""
        };
        Value::from_file(self.output().join(format!("{contents}Info.plist")))?
            .into_dictionary()
            .context("the plist is a dictionary")
    }

    /// The log lines of the fake tools that contain `text`.
    fn logged(&self, text: &str) -> Result<Vec<String>> {
        Ok(fs::read_to_string(self.log())?
            .lines()
            .filter(|line| line.contains(text))
            .map(str::to_owned)
            .collect())
    }

    /// An Icon Composer package.
    fn icon(&self) -> Result<PathBuf> {
        let icon = self.root.path().join("Brand.icon");
        fs::create_dir_all(&icon)?;
        fs::write(icon.join("icon.json"), "{}")?;
        Ok(icon)
    }
}

/// The SDK and compile triple of `target` at the pack's minimum OS version.
fn compile_target(target: &str) -> Result<(&'static str, String)> {
    let minimum = |platform: &str| -> Result<String> {
        let path = Path::new(BUILD).join(format!("{platform}-deployment-target"));
        Ok(fs::read_to_string(path)?.trim_end().to_owned())
    };
    Ok(match target {
        "macos-arm64" => ("macosx", format!("arm64-apple-macos{}", minimum("macos")?)),
        "macos-x64" => ("macosx", format!("x86_64-apple-macos{}", minimum("macos")?)),
        "ios-arm64" => ("iphoneos", format!("arm64-apple-ios{}", minimum("ios")?)),
        "ios-simulator-arm64" => (
            "iphonesimulator",
            format!("arm64-apple-ios{}-simulator", minimum("ios")?),
        ),
        "ios-simulator-x64" => (
            "iphonesimulator",
            format!("x86_64-apple-ios{}-simulator", minimum("ios")?),
        ),
        _ => bail!("unsupported Apple test target: {target}"),
    })
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().context("test paths are UTF-8")
}

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::create_dir_all(path.parent().context("executable path has a parent")?)?;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn run(command: &mut Command) -> Result<()> {
    let output = command.output()?;
    if !output.status.success() {
        bail!(
            "{command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn failure(command: &mut Command) -> Result<String> {
    let Output { status, stderr, .. } = command.output()?;
    if status.success() {
        bail!("{command:?} succeeded");
    }
    Ok(String::from_utf8(stderr)?)
}

fn string<'a>(plist: &'a Dictionary, key: &str) -> Option<&'a str> {
    plist.get(key).and_then(Value::as_string)
}

fn ios_minimum() -> Result<String> {
    Ok(
        fs::read_to_string(Path::new(BUILD).join("ios-deployment-target"))?
            .trim_end()
            .to_owned(),
    )
}

fn macos_minimum() -> Result<String> {
    Ok(
        fs::read_to_string(Path::new(BUILD).join("macos-deployment-target"))?
            .trim_end()
            .to_owned(),
    )
}

/// Stages the plugin `id` in `input`, a Swift package whose module `Module`,
/// with the one source `Plugin.swift`, defines `Class`, its class
/// `Module.Class`. Returns the package's directory.
fn stage_plugin(input: &Path, id: &str, class: &str) -> Result<PathBuf> {
    let plugin = input.join("plugins").join(id);
    let (module, name) = class
        .split_once('.')
        .context("the class names its module")?;
    let package = plugin.join("native");
    let (ios, macos) = (ios_minimum()?, macos_minimum()?);
    fs::create_dir_all(package.join("Sources").join(module))?;
    fs::write(plugin.join("class"), class)?;
    fs::write(
        package.join("Package.swift"),
        format!(
            r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "{module}",
  platforms: [.iOS("{ios}"), .macOS("{macos}")],
  products: [.library(name: "{module}", targets: ["{module}"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(name: "{module}", dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
"#
        ),
    )?;
    fs::write(
        package.join("Sources").join(module).join("Plugin.swift"),
        format!(
            r#"import TokamakPlugin

public final class {name}: TokamakPlugin {{
  public let id = "{id}"
  public init(host: TokamakHost) {{}}
}}
"#
        ),
    )?;
    Ok(package)
}

#[test]
fn packages_the_app_and_its_plugins_into_the_macos_bundle() -> Result<()> {
    let fixture = Fixture::new("macos-arm64")?;
    // Each plugin is its own module, so their sources may share a file name.
    for (id, class) in [
        ("location", "TokamakLocation.TokamakLocationPlugin"),
        ("echo", "TokamakEcho.TokamakEchoPlugin"),
        ("ping", "TokamakPing.TokamakPingPlugin"),
    ] {
        stage_plugin(&fixture.input(), id, class)?;
    }
    // A plugin's package resources reach the bundle, where `Bundle.module` finds them.
    let plugin = fixture.input().join("plugins/location");
    let manifest = fs::read_to_string(plugin.join("native/Package.swift"))?;
    fs::write(
        plugin.join("native/Package.swift"),
        manifest.replace(
            r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])"#,
            r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")], resources: [.copy("regions.json")])"#,
        ),
    )?;
    fs::write(
        plugin.join("native/Sources/TokamakLocation/regions.json"),
        "[]",
    )?;
    fs::write(
        plugin.join("plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSLocationUsageDescription</key>
	<string>Finds you</string>
</dict>
</plist>
"#,
    )?;

    run(&mut fixture.build())?;

    let bundle = fixture.output();
    assert!(bundle.join("Contents/MacOS/demo-app").is_file());
    assert_eq!(
        fs::read_to_string(bundle.join("Contents/Resources/app/index.html"))?,
        "<html></html>"
    );
    let plist = fixture.plist()?;
    for (key, value) in [
        ("CFBundleName", "Demo App"),
        ("CFBundleExecutable", "demo-app"),
        ("CFBundleIdentifier", "com.example.demo"),
        ("CFBundleShortVersionString", "1.2.3"),
        ("CFBundleVersion", "1.2.3"),
        ("TokamakHost", "demo-app.tokamak.local"),
    ] {
        assert_eq!(string(&plist, key), Some(value), "{key}");
    }
    assert!(plist.contains_key("NSLocationUsageDescription"));
    assert!(!plist.contains_key("CFBundleIconName"));
    assert!(!bundle.join("Contents/Resources/Assets.car").exists());
    let mut resource_bundles = Vec::new();
    for entry in fs::read_dir(bundle.join("Contents/Resources"))? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.ends_with(".bundle") {
            resource_bundles.push(name);
        }
    }
    assert_eq!(resource_bundles, ["TokamakLocation_TokamakLocation.bundle"]);
    Ok(())
}

#[test]
fn pins_the_versions_of_plugins_dependencies_in_the_lock_file() -> Result<()> {
    let fixture = Fixture::new("macos-arm64")?;
    // A package that a plugin depends on by URL, as on a remote host.
    let greeting = fixture.root.path().join("greeting");
    fs::create_dir_all(greeting.join("Sources/Greeting"))?;
    fs::write(
        greeting.join("Package.swift"),
        format!(
            r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "Greeting",
  platforms: [.iOS("{}"), .macOS("{}")],
  products: [.library(name: "Greeting", targets: ["Greeting"])],
  targets: [.target(name: "Greeting")]
)
"#,
            ios_minimum()?,
            macos_minimum()?
        ),
    )?;
    let release = |version: &str| -> Result<()> {
        fs::write(
            greeting.join("Sources/Greeting/Greeting.swift"),
            format!("public let greeting = \"{version}\"\n"),
        )?;
        for arguments in [
            &["add", "."][..],
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                version,
            ],
            &["tag", version],
        ] {
            run(Command::new("git").arg("-C").arg(&greeting).args(arguments))?;
        }
        Ok(())
    };
    run(Command::new("git").args(["init", "-q"]).arg(&greeting))?;
    release("1.0.0")?;
    // The lock file records the location as SwiftPM resolves it, without links.
    let greeting = fs::canonicalize(&greeting)?;
    let plugin = stage_plugin(&fixture.input(), "hello", "TokamakHello.TokamakHelloPlugin")?;
    let manifest = fs::read_to_string(plugin.join("Package.swift"))?;
    fs::write(
        plugin.join("Package.swift"),
        manifest
            .replace(
                "dependencies: [.package(path:",
                &format!(
                    "dependencies: [.package(url: \"file://{}\", from: \"1.0.0\"), .package(path:",
                    path_text(&greeting)?
                ),
            )
            .replace(
                r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")]"#,
                r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin"), .product(name: "Greeting", package: "greeting")]"#,
            ),
    )?;
    let lock = fixture.input().join("lock/Package.resolved");
    let built = |page: &str| -> Result<String> {
        fs::write(fixture.input().join("app/index.html"), page)?;
        run(&mut fixture.build())?;
        Ok(fs::read_to_string(&lock)?)
    };

    let first = built("<html>1</html>")?;
    assert!(first.contains("\"1.0.0\""), "{first}");

    // A newer release leaves the build at the pinned version until the lock file goes.
    release("1.1.0")?;
    assert!(built("<html>2</html>")?.contains("\"1.0.0\""));
    fs::remove_file(&lock)?;
    assert!(built("<html>3</html>")?.contains("\"1.1.0\""));
    // A lock file that pins another version, such as an earlier commit's, is followed.
    fs::write(&lock, &first)?;
    assert!(built("<html>4</html>")?.contains("\"1.0.0\""));
    Ok(())
}

#[test]
fn signs_the_dynamic_frameworks_of_plugins_dependencies_in_the_bundle() -> Result<()> {
    for (target, frameworks) in [
        ("macos-arm64", "Contents/Frameworks"),
        ("ios-simulator-arm64", "Frameworks"),
    ] {
        let fixture = Fixture::new(target)?;
        let shared = fixture.root.path().join("shared");
        fs::create_dir_all(shared.join("Sources/Shared"))?;
        fs::write(
            shared.join("Package.swift"),
            format!(
                r#"// swift-tools-version: 5.9
import PackageDescription

let package = Package(
  name: "Shared",
  platforms: [.iOS("{}"), .macOS("{}")],
  products: [.library(name: "Shared", type: .dynamic, targets: ["Shared"])],
  targets: [.target(name: "Shared")]
)
"#,
                ios_minimum()?,
                macos_minimum()?
            ),
        )?;
        fs::write(
            shared.join("Sources/Shared/Shared.swift"),
            "public let shared = 1\n",
        )?;
        let plugin = stage_plugin(&fixture.input(), "hello", "TokamakHello.TokamakHelloPlugin")?;
        let manifest = fs::read_to_string(plugin.join("Package.swift"))?;
        fs::write(
            plugin.join("Package.swift"),
            manifest
                .replace(
                    "dependencies: [.package(path:",
                    &format!(
                        "dependencies: [.package(path: \"{}\"), .package(path:",
                        path_text(&shared)?
                    ),
                )
                .replace(
                    r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")]"#,
                    r#"dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin"), .product(name: "Shared", package: "shared")]"#,
                ),
        )?;
        fs::write(
            plugin.join("Sources/TokamakHello/Shared.swift"),
            "import Shared\n\nlet hello = shared\n",
        )?;

        run(&mut fixture.build())?;

        let bundle = fixture.output();
        assert!(
            bundle.join(frameworks).join("Shared.framework").is_dir(),
            "{target}"
        );
        // The pack's signing tool is faked for macOS; the real one signs
        // ad hoc without a team.
        if fixture.platform() == "macos" {
            run(Command::new(TOOL)
                .args(["sign", "--platform", "macos", "--project"])
                .arg(fixture.root.path())
                .arg("--bundle")
                .arg(&bundle)
                .args(["--bundle-id", "com.example.demo"])
                .env_remove("TOKAMAK_MACOS_TEAM_ID")
                .env_remove("TOKAMAK_MACOS_ENTITLEMENTS"))?;
        }
        run(Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&bundle))?;
    }
    Ok(())
}

#[test]
fn links_only_the_runtime_entry_points_the_app_uses() -> Result<()> {
    for target in ["macos-arm64", "macos-x64"] {
        let fixture = Fixture::new(target)?;
        let executable = fixture.output().join("Contents/MacOS/demo-app");
        let assert_exports_storage = |symbols: &str, exported: bool| -> Result<()> {
            fs::write(fixture.input().join("metadata/exported-symbols"), symbols)?;
            run(&mut fixture.build())?;
            let nm = Command::new("nm").arg("-gU").arg(&executable).output()?;
            assert_eq!(
                String::from_utf8(nm.stdout)?.contains("_tokamak_storage"),
                exported,
                "{target} exporting {symbols:?}"
            );
            Ok(())
        };

        assert_exports_storage("", false)?;
        assert_exports_storage("tokamak_storage\n", true)?;
    }
    Ok(())
}

#[test]
fn embeds_the_entitlements_in_ios_simulator_apps() -> Result<()> {
    for target in ["ios-simulator-arm64", "ios-simulator-x64"] {
        let fixture = Fixture::new(target)?;
        let entitlements = fixture.root.path().join("App.entitlements");
        fs::write(
            &entitlements,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>aps-environment</key><string>development</string></dict></plist>"#,
        )?;

        run(fixture
            .build()
            .env("TOKAMAK_IOS_ENTITLEMENTS", &entitlements))?;

        let plist = fixture.plist()?;
        assert_eq!(
            plist
                .get("CFBundleSupportedPlatforms")
                .and_then(Value::as_array)
                .and_then(|platforms| platforms.first())
                .and_then(Value::as_string),
            Some("iPhoneSimulator")
        );
        assert_eq!(
            string(&plist, "MinimumOSVersion"),
            Some(ios_minimum()?.as_str())
        );
        assert_eq!(
            simulator_entitlements(&fixture.output().join("demo-app"))?,
            serde_json::json!({
                "application-identifier": "com.example.demo",
                "keychain-access-groups": ["com.example.demo"],
                "aps-environment": "development",
            })
        );
    }
    Ok(())
}

/// The entitlements the simulator reads from the executable's
/// `__TEXT,__entitlements` section.
fn simulator_entitlements(executable: &Path) -> Result<serde_json::Value> {
    let section = executable.with_extension("entitlements");
    run(Command::new("xcrun")
        .arg("segedit")
        .arg(executable)
        .args(["-extract", "__TEXT", "__entitlements"])
        .arg(&section))?;
    let output = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&section)
        .output()?;
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn signs_physical_ios_apps_with_their_build_number() -> Result<()> {
    let fixture = Fixture::new("ios-arm64")?;

    run(fixture
        .fake_build()?
        .env("TOKAMAK_IOS_BUILD_NUMBER", "5")
        .env("TOKAMAK_IOS_SIGNING_IDENTITY", "Apple Development: Test"))?;

    assert!(fixture.output().join("demo-app").is_file());
    let plist = fixture.plist()?;
    assert_eq!(
        plist.get("LSRequiresIPhoneOS").and_then(Value::as_boolean),
        Some(true)
    );
    assert_eq!(string(&plist, "CFBundleVersion"), Some("5"));
    assert_eq!(string(&plist, "CFBundleShortVersionString"), Some("1.2.3"));
    assert_eq!(
        fixture.logged("sign ")?,
        ["sign Apple Development: Test {}"]
    );
    Ok(())
}

#[test]
fn compiles_the_icon_package() -> Result<()> {
    for (target, assets, sdk, minimum) in [
        (
            "macos-arm64",
            "Contents/Resources/Assets.car",
            "macosx",
            "macos-deployment-target",
        ),
        (
            "ios-simulator-arm64",
            "Assets.car",
            "iphonesimulator",
            "ios-deployment-target",
        ),
    ] {
        let fixture = Fixture::new(target)?;
        let icon = fixture.icon()?;

        run(fixture.fake_build()?.env(fixture.variable("ICON"), &icon))?;

        assert!(fixture.output().join(assets).is_file());
        assert_eq!(
            string(&fixture.plist()?, "CFBundleIconName"),
            Some("AppIcon")
        );
        let minimum = fs::read_to_string(Path::new(BUILD).join(minimum))?;
        let [actool] = fixture
            .logged(" actool ")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("actool did not run once"))?;
        assert!(actool.contains(&format!(
            "--platform {sdk} --minimum-deployment-target {} --app-icon AppIcon",
            minimum.trim_end()
        )));
        assert!(actool.ends_with("/AppIcon.icon"));
    }
    Ok(())
}

#[test]
fn rejects_invalid_icons_plugin_classes_and_build_numbers() -> Result<()> {
    let fixture = Fixture::new("macos-arm64")?;
    let icon = fixture.root.path().join("AppIcon.png");
    fs::create_dir(&icon)?;
    assert!(
        failure(fixture.build().env("TOKAMAK_MACOS_ICON", &icon))?
            .contains("macos.icon must be an Icon Composer .icon package")
    );

    assert!(
        failure(
            fixture
                .fake_build()?
                .env("TOKAMAK_MACOS_BUILD_NUMBER", "0.1.2-5")
        )?
        .contains(
            "Apple build number must contain one to three period-separated integers: 0.1.2-5"
        )
    );

    let plugin = stage_plugin(
        &fixture.input(),
        "location",
        "TokamakLocation.TokamakLocationPlugin",
    )?;
    let class = fixture.input().join("plugins/location/class");
    for invalid in ["Location-Plugin", "TokamakLocationPlugin"] {
        fs::write(&class, invalid)?;
        let message = failure(&mut fixture.fake_build()?)?;
        assert!(
            message.contains(&format!(
                "plugin 'location' has invalid macos class '{invalid}'; name it with its module, as Module.Class"
            )),
            "{message}"
        );
    }

    fs::write(&class, "TokamakLocation.TokamakLocationPlugin")?;
    fs::remove_file(plugin.join("Package.swift"))?;
    assert!(
        failure(&mut fixture.fake_build()?)?
            .contains("plugin 'location' has no Swift package, apple/Package.swift")
    );
    Ok(())
}

#[test]
fn reuses_the_bundle_while_only_the_environment_and_signing_settings_change() -> Result<()> {
    for target in ["macos-arm64", "ios-arm64", "ios-simulator-arm64"] {
        let fixture = Fixture::new(target)?;
        let icon = fixture.icon()?;
        let build = |environment: &str, build_number: &str, team: &str| -> Result<()> {
            fs::write(
                fixture.input().join("app/worker-environment.json"),
                environment,
            )?;
            run(fixture
                .fake_build()?
                .env(fixture.variable("ICON"), &icon)
                .env(fixture.variable("BUILD_NUMBER"), build_number)
                .env(fixture.variable("TEAM_ID"), team))
        };

        build(r#"{"API":"test"}"#, "1", "TEAM")?;
        build(r#"{"API":"production"}"#, "2", "OTHER")?;

        assert_eq!(fixture.logged("xcodebuild build ")?.len(), 1, "{target}");
        assert_eq!(fixture.logged(" actool ")?.len(), 1, "{target}");
        let signatures = fixture.logged("sign ")?;
        assert_eq!(signatures.len(), 2, "{target}");
        assert!(
            signatures[1].ends_with(r#" {"API":"production"}"#),
            "{target}"
        );
        assert_eq!(
            signatures[1].contains("OTHER"),
            target != "ios-simulator-arm64",
            "{target}"
        );
        let plist = fixture.plist()?;
        assert_eq!(string(&plist, "CFBundleVersion"), Some("2"), "{target}");
        assert_eq!(
            string(&plist, "CFBundleIconName"),
            Some("AppIcon"),
            "{target}"
        );

        // A changed pack rebuilds the bundle.
        fs::write(fixture.pack().join("native-shell/Extra.swift"), "")?;
        build(r#"{"API":"production"}"#, "2", "OTHER")?;
        assert_eq!(fixture.logged("xcodebuild build ")?.len(), 2, "{target}");
    }
    Ok(())
}

#[test]
fn lists_the_signing_assets() -> Result<()> {
    let fixture = Fixture::new("ios-arm64")?;
    let output = Command::new("bash")
        .args(["build/entrypoint", "certs"])
        .current_dir(fixture.pack())
        .output()?;

    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)?.starts_with("iOS signing identities"));
    Ok(())
}

#[test]
fn explains_conflicting_automatic_and_manual_signing() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let message = failure(
        Command::new(TOOL)
            .args(["sign", "--platform", "ios", "--project"])
            .arg(directory.path())
            .arg("--bundle")
            .arg(directory.path().join("Demo.app"))
            .args(["--bundle-id", "com.example.demo"])
            .env_remove("TOKAMAK_IOS_ENTITLEMENTS")
            .env("TOKAMAK_IOS_TEAM_ID", "TEAM")
            .env("TOKAMAK_IOS_SIGNING_IDENTITY", "IDENTITY_SHA1")
            .env(
                "TOKAMAK_IOS_PROVISIONING_PROFILE",
                directory.path().join("manual.mobileprovision"),
            ),
    )?;

    for line in [
        "automatic and manual iOS signing cannot be combined.",
        "Automatic: set TOKAMAK_IOS_TEAM_ID or use --ios-team-id TEAM_ID",
        "Manual:    set BOTH TOKAMAK_IOS_SIGNING_IDENTITY and",
    ] {
        assert!(message.contains(line), "{message}");
    }
    Ok(())
}
