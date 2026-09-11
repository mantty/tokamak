# tokamak

**tokamak** is a Cloudflare Worker compatible runtime for building cross-platform apps.

Create Android, iOS, macOS, Windows, and native web applications from a single codebase using full-stack JS frameworks.

> tokamak is in early alpha. Expect breaking changes.

## Install

On macOS, Linux, or WSL:

```sh
curl -fsSL https://raw.githubusercontent.com/mantty/tokamak/main/scripts/install.sh | bash
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/mantty/tokamak/main/scripts/install.ps1 | iex
```

The installer downloads the newest published release, including pre-releases,
and installs the CLI and every target pack under `~/.local`. It prints the PATH
change when `~/.local/bin` is not already available.
It optionally uses `GH_TOKEN` or `GITHUB_TOKEN` to authenticate the GitHub
release lookup, and otherwise keeps using the unauthenticated lookup.

tokamak doesn't add any external dependencies, but you will need the toolchain for any platforms you wish to build for:

| Platform | Requirements |
| --- | --- |
| macOS and iOS | macOS with Xcode; physical iOS devices must be registered for development in Xcode |
| Android | Android SDK 35, Java 17, Gradle, and Bash |
| Windows | 64-bit Windows |

## Build an application

We do not (currently) support all Cloudflare bindings to additional services they offer, but by and large a basic fullstack web-app written for Cloudflare workers is all you need to build a native app.

Your project must have a `package.json` build script and a Wrangler config with
at least `name` and `main`. tokamak runs the build with pnpm, Yarn, or npm based on
the project's lockfile, then writes the native bundle under `build/<platform>`.
The optional Tokamak configuration file is described below.

```sh
tok build macos --project ./my-app
```

Use `-w` or `--wrangler` to point to a specific Wrangler config file:

```sh
tok build macos --wrangler dist/server/wrangler.json
```

Platforms are `android`, `ios`, `ios-simulator`, `macos`, and `windows`.
Multiple platforms can be comma-separated, for example `macos,android`.

### Tokamak configuration

tokamak looks for `tokamak.jsonc` in the current directory, followed by
`tokamak.json`. The file is optional. Use `-c` or `--config` to provide a
different file or directory; the option defaults to the current directory.
JSONC comments and trailing commas are supported, and plain JSON is also valid.

The supported values are `name`, `identifier`, `version`, and `icons`:

```jsonc
{
  "name": {
    "default": "My App",
    "ios": "Myapp Pro",
  },
  "identifier": {
    "default": "com.example.myapp",
    "ios": "com.example.myapp.ios",
  },
  "version": "1.0.0",
  "icons": {
    "android": "assets/icons/android",
    "ios": "assets/icons/AppIcon.icon",
    "macos": "assets/icons/AppIcon.icon",
    "windows": "assets/icons/windows/AppIcon.ico",
  },
}
```

The `name` value is optional, but `name.default` is required when it is
present. Platform names are optional and fall back to `default`; `ios` is
used for both iOS devices and iOS simulators. Names retain their spelling and
capitalization for display. Tokamak derives a lower-case ASCII slug for bundle
filenames, application IDs, and `tokamak.local` hosts, so `My App` becomes
`my-app`. If `name` is absent, the Wrangler Worker name is used. The slug is
also used to derive an identifier when no identifier is configured.

The `identifier` value is optional, but `identifier.default` is required when
it is present. Platform identifiers are optional and fall back to `default`.
They are used as the Apple bundle identifier and Android application ID. If
`identifier` is absent, Tokamak keeps deriving the identifier from the
application slug. `TOKAMAK_IDENTIFIER` overrides the configured
value, and `TOKAMAK_ANDROID_IDENTIFIER`, `TOKAMAK_IOS_IDENTIFIER`,
`TOKAMAK_MACOS_IDENTIFIER`, or `TOKAMAK_WINDOWS_IDENTIFIER` override it for
one platform. iOS simulators use the iOS variable.

The `version` value is optional in the configuration, but is required for
`tok build`. Set it in the configuration or with `TOKAMAK_VERSION`; the
environment variable takes precedence. Development builds keep their existing
native default when no version is supplied. Apple builds use the value for
`CFBundleShortVersionString` and, by default, `CFBundleVersion`; Android uses it
as `versionName`.

Apple target packs accept an optional per-build number through target-pack
variables. Use `ios-build-number` for iOS (including the simulator) and
`macos-build-number` for macOS:

```sh
tok build ios --set ios-build-number=5
TOKAMAK_MACOS_BUILD_NUMBER=7 tok build macos
```

These map to `TOKAMAK_IOS_BUILD_NUMBER` and `TOKAMAK_MACOS_BUILD_NUMBER`.
Values must contain one to three period-separated integers. If omitted, the
target pack continues to use the app version as the Apple build number.

Apple target packs also accept an optional user-provided application plist.
Set `ios-plist` for iOS devices and simulators, or `macos-plist` for macOS:

```sh
tok build ios --set ios-plist=native/Info.plist
TOKAMAK_MACOS_PLIST=native/Info.plist tok build macos
```

The corresponding environment variables are `TOKAMAK_IOS_PLIST` and
`TOKAMAK_MACOS_PLIST`. Relative paths are resolved from the project directory.
The file may be XML or binary and must have a dictionary at its root. It is
optional; when present, Tokamak overlays its generated application values on
top of it, so Tokamak's identifiers, names, versions, platform metadata, and
defaults take precedence. User-defined plist values that Tokamak does not
generate are preserved.

Each icon platform entry is optional. If the Tokamak configuration or a
platform entry is absent, that platform keeps its existing icon behavior.

- `android` points to the contents of an Android `res` directory. It should
  contain launcher resources such as `mipmap-*/ic_launcher`.
- `ios` points to an Apple Icon Composer `.icon` package. It is used for both
  iOS devices and iOS simulators.
- `macos` points to an Apple Icon Composer `.icon` package. The same package
  can be used for both iOS and macOS, or each platform can use its own package.
- `windows` points to an `.ico` file. It is copied beside the packaged
  executable and used for the Windows window and taskbar icon.

The Apple `.icon` package must be created by Icon Composer and kept as a
package directory; do not point to a flattened export. Apple builds compile
the package into the platform bundle.

Relative paths are resolved from the directory containing the Tokamak
configuration file. Invalid paths or platform formats fail the native build.

Physical iOS builds use automatic development signing by default. Tokamak
selects an available Apple Development team automatically. If the choice is
ambiguous, it lists the team names from the signing certificates and their
IDs. Choose the team that owns your app. Set its ID for your shell and rerun
your command:

```sh
export TOKAMAK_IOS_TEAM_ID=YOUR_TEAM_ID
```

Alternatively, select the team for a single command with `--set`:

```sh
tok dev DEVICE_ID --set ios-team-id=YOUR_TEAM_ID -- pnpm dev
tok build ios --set ios-team-id=YOUR_TEAM_ID
```

`--set` may be repeated for multiple target-pack variables. A value such as
`ios-team-id=YOUR_TEAM_ID` is passed to the iOS target pack as
`TOKAMAK_IOS_TEAM_ID`.

Tokamak selects the signing identity and provisioning profile for that team,
asking Xcode to provision the app when needed. No manual signing variables
are required. If a certificate has no team name, the list identifies its
signing identity instead and marks the team name as unavailable.

For **manual signing**, provide both the identity and its matching profile.
List locally installed iOS signing assets on macOS with:

```sh
tok certs
```

This read-only command lists identity names and SHA-1 selectors, plus profile
names, paths, team IDs, app identifiers, expiration dates, and matching installed
identities. It includes development and distribution profiles; matching means
the identity's certificate is included in the profile, not that the pair is valid
for every app or device. It does not create or import signing assets.

Use the identity's SHA-1 and the profile's path. They can be set for the shell:

```sh
export TOKAMAK_IOS_SIGNING_IDENTITY="IDENTITY_SHA1"
export TOKAMAK_IOS_PROVISIONING_PROFILE="/path/to/profile.mobileprovision"
```

Or set both for a single command:

```sh
tok dev DEVICE_ID \
  --set ios-signing-identity=IDENTITY_SHA1 \
  --set ios-provisioning-profile=/path/to/profile.mobileprovision \
  -- pnpm dev
tok build ios \
  --set ios-signing-identity=IDENTITY_SHA1 \
  --set ios-provisioning-profile=/path/to/profile.mobileprovision
```

An explicit team (`TOKAMAK_IOS_TEAM_ID` or `--set ios-team-id=TEAM_ID`) and the
manual pair are mutually exclusive. Providing both produces an error explaining
the two choices. The Apple target pack owns the signing and provisioning
selection; `tok` passes target-pack variables through unchanged.

Both `tok dev` and `tok build ios` use these settings; the latter provisions
for a generic iOS device and does not require a device ID. iOS Simulator
builds do not require provisioning.

## Development

tokamak supports dev mode with HMR via wrangler and vite, proxying to a device for native capabilities.
Dev mode is significantly less performant than a real build, but provides an excellent local dev loop.

To list available local devices, simulators, and emulators for dev mode:

```sh
tok devices
```

Pass a device selector and the framework's development command to `tok dev`:

```sh
tok dev macos --project ./my-app -- pnpm dev
```

By default tokamak expects your server to available on `http://localhost:5173` (vite's default port). Use `--server` when the framework uses another port.

## Example

[The Astro example](examples/astro) exercises server rendering, static assets,
navigation, WebSockets, and the native geolocation plugin.

```sh
pnpm --dir examples/astro install --frozen-lockfile
tok dev macos --project examples/astro -- pnpm dev
```

To produce a native bundle instead:

```sh
pnpm --dir examples/astro run build
tok build macos \
  --project examples/astro \
  --wrangler examples/astro/dist/server/wrangler.json \
  --skip-project-build
```

## Contributing

### Requirements

- Rust 1.96 through `rustup`; the repository selects it with
  `rust-toolchain.toml`.
- Node.js 22 and pnpm 9.9.
- CMake, Clang and libclang, Perl, and the native C/C++ toolchain for your host.

Target-pack work additionally requires:

| Target | Requirements |
| --- | --- |
| Apple | macOS with Xcode |
| Android | Android SDK 35, NDK `29.0.14206865`, Java 17, Gradle, and Bash |
| Windows | 64-bit Windows with Visual Studio 2022 C++ Build Tools and NASM |

Install the JavaScript dependencies once after cloning:

```sh
pnpm --dir plugins install --frozen-lockfile
pnpm --dir tools/esbuild-hosts install --frozen-lockfile
pnpm --dir examples/astro install --frozen-lockfile
```

### Build the CLI

```sh
cargo build --release -p tokamak-cli
```

The executable is written to `target/release/tok` (`tok.exe` on Windows).

### Build a target pack

Install the Rust target, then build the runtime, native shell, and runtime tools
for one target. Apple target packs also need both macOS host targets because the
pack includes a universal host-side signing tool:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo run -p xtask -- target-pack --target macos-arm64
```

Target packs are written to `target/tokamak-target-packs/<target>`. Run
`cargo run -p tokamak-cli -- targets` to list supported targets.

### Package the plugins

```sh
pnpm --dir plugins --filter '@tokamak/*' exec pnpm pack --pack-destination "$PWD/artifacts"
```

### Build the example against local sources

After building a target pack:

```sh
pnpm --dir examples/astro run build
cargo run -p tokamak-cli -- build macos \
  --project examples/astro \
  --wrangler examples/astro/dist/server/wrangler.json \
  --target-pack target/tokamak-target-packs/macos-arm64 \
  --skip-project-build
```

Use the local CLI in development mode with:

```sh
cargo run -p tokamak-cli -- dev macos \
  --project examples/astro \
  --target-pack target/tokamak-target-packs/macos-arm64 \
  -- pnpm dev
```

### Checks

Runtime contract tests execute the same Worker in packaged QuickJS and pinned
Cloudflare `workerd` with `nodejs_compat` and compatibility date `2026-08-25`.
Node hosts the test runner; it is not
the compatibility reference.

Run the common checks before submitting a change:

```sh
pnpm --dir tools/esbuild-hosts install --frozen-lockfile
pnpm --dir tokamak/tests/quickjs_runtime install --frozen-lockfile
cargo fmt --all --check
cargo test -p tokamak --features native
node --test tokamak/tests/quickjs_runtime/runtime.test.mjs
cargo test -p tokamak-cli --lib --bin tok --test cli --test target_pack
cargo test -p xtask
pnpm --dir plugins lint:ts
pnpm --dir plugins test:ts
```

Platform-specific lint and build-test commands are kept in
[the Checks workflow](.github/workflows/test.yaml).

### Packaged Worker runtime

Each request owns a fresh QuickJS runtime, module graph, and temporary filesystem.
The native runtime installs Web globals before evaluating application modules.
Supported builtin imports resolve to runtime-owned modules; they are not bundled
into the application. Builtin JavaScript is precompiled to bytecode when the
runtime is built and embedded in its native library. Application builds use the
prebuilt runtime from the target pack. Static imports, dynamic imports, and
`process.getBuiltinModule()` share the same implementations within a request.

`cloudflare:workers` exposes `env` and `waitUntil`. Its `waitUntil` uses the same
request task queue as the handler's execution context. Runtime implementation
modules are private. Unsupported builtin imports fail during module loading;
`scheduler.wait()` and `passThroughOnException()` report that they are unsupported.

Text decoding uses WHATWG encoding labels, streaming state, BOM handling, and
fatal/replacement modes. Random bytes come from the operating system.
The contract suite covers these APIs, Base64, binary bodies, and builtin identity;
it is not a claim of complete Workers API coverage. WebAssembly is unsupported.
Tokamak does not emulate historical Cloudflare compatibility-date modes.
