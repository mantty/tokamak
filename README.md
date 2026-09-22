# tokamak

**tokamak** is a Cloudflare Worker compatible runtime for building cross-platform apps.

Create Android, iOS, macOS, Windows, and native web applications from a single codebase using full-stack JS frameworks.

> tokamak is in early alpha. Expect breaking changes.

## Install

Add the `tok` CLI to your project:

```sh
npm install --save-dev @tokamakdev/tok
```

With pnpm, use `pnpm add -D @tokamakdev/tok`. Run it with `npx tok`
(`pnpm exec tok`) or from a `package.json` script; the examples in this README
write `tok`.

The package installs the `tok` binary for your machine and the platform packs
it can build. Platform packs contain the tokamak runtime and native shell for
one target:

| Machine | Platform packs |
| --- | --- |
| macOS, Apple silicon | `android-arm64`, `ios-arm64`, `ios-simulator-arm64`, `macos-arm64` |
| macOS, Intel | `android-arm64`, `ios-arm64`, `ios-simulator-x64`, `macos-x64` |
| Windows x64 | `android-arm64`, `windows-x64` |
| Linux x64 | `android-arm64` |

Each platform pack is an optional dependency named
`@tokamakdev/platform-<target>`, so an install that omits optional
dependencies (`--omit=optional`, `--no-optional`) leaves them out.

### Installer script

If you do not use npm, the installer script installs `tok` and every platform
pack for your user account. On macOS, Linux, or WSL:

```sh
curl -fsSL https://raw.githubusercontent.com/mantty/tokamak/main/scripts/install.sh | bash
```

On Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/mantty/tokamak/main/scripts/install.ps1 | iex
```

The installer downloads the newest published release, including pre-releases,
and installs the CLI and every platform pack under `~/.local`. It prints the PATH
change when `~/.local/bin` is not already available.
It optionally uses `GH_TOKEN` or `GITHUB_TOKEN` to authenticate the GitHub
release lookup, and otherwise keeps using the unauthenticated lookup.

### Platform pack lookup

`tok` uses the first of:

1. the directory passed with `--platform-pack`;
2. `TOKAMAK_PLATFORM_PACK_PATH`, a list of directories that each contain
   `<target>/platform-pack.json`, separated by `:` (`;` on Windows);
3. `platform-packs` directories installed alongside the `tok` executable;
4. `~/.local/share/tokamak/platform-packs`, where the installer places them.

When `tok` runs from the npm package, its launcher sets
`TOKAMAK_PLATFORM_PACK_PATH` to the platform packs installed with it, unless
the variable is already set.

### Platform toolchains

tokamak doesn't add any external dependencies, but you will need the toolchain for any platforms you wish to build for:

| Platform | Requirements |
| --- | --- |
| macOS and iOS | macOS with Xcode; physical iOS devices must be registered for development in Xcode |
| Android | Android SDK 35, Java 17, Gradle, and Bash |
| Windows | 64-bit Windows |

## AI assistant plugin

This repository includes a repo-installable, skill-only plugin for Codex and
Claude Code. See [the plugin README](ai/plugin/README.md) for installation and
testing instructions.

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

### Wrangler environments and build reuse

`--env NAME` selects `env.NAME.vars` from `wrangler.json`, `.jsonc`, or `.toml`;
omit it for top-level `vars`. Named vars do not inherit top-level vars. Values
can be strings or JSON.

```jsonc
{
  "name": "my-app",
  "main": "dist/server/entry.mjs",
  "vars": { "API_URL": "https://api.example.com" },
  "env": {
    "test": { "vars": { "API_URL": "https://test-api.example.com", "OPTIONS": { "preview": true } } },
    "production": { "vars": { "API_URL": "https://api.example.com" } }
  }
}
```

```sh
tok build ios --env test
tok build ios --env production
```

For generated Wrangler files, pass `--wrangler PATH`; `userConfigPath` supplies
vars from the source config. Never put secrets in packaged `vars`.

`build/` holds output and reusable build data; `--build-dir PATH` moves both.
An empty or stale cache builds normally. Changing only vars on Apple updates
the bundled environment file and re-signs without recompiling the shell.

GitHub Actions: use these steps in both a merge build and a later promotion
workflow on the default branch. Promotion takes the tested commit as a
`workflow_dispatch` input named `sha` and uses `--env production` in the final step.

```yaml
- uses: actions/checkout@v6
  with:
    ref: ${{ inputs.sha || github.sha }}
- uses: actions/cache@v4
  with:
    path: build/
    key: tokamak-ios-${{ runner.os }}-${{ inputs.sha || github.sha }}
- run: tok build ios --env test
```

Install `tok` and project dependencies before the build step; upload the signed
output separately.

### Tokamak configuration

tokamak looks for `tokamak.jsonc` in the current directory, followed by
`tokamak.json`. The file is optional. Use `-c` or `--config` to provide a
different file or directory; the option defaults to the current directory.
JSONC comments and trailing commas are supported, and plain JSON is also valid.

The supported values are `name`, `identifier`, `icon`, and `version`. Top-level
values are defaults; a platform object (`android`, `ios`, `macos`, `windows`)
overrides `name`, `identifier`, or `icon` for that platform. `version` is
top-level only. `ios` covers iOS devices and simulators. Unknown keys are
rejected.

```jsonc
{
  // Defaults for every platform
  "name": "My App",
  "identifier": "com.example.myapp",
  "icon": "assets/icons/AppIcon.icon",
  "version": "1.0.0",

  // Platform overrides
  "ios": {
    "name": "Myapp Pro",
    "identifier": "com.example.myapp.ios",
  },
  "android": {
    "icon": "assets/icons/android",
  },
  "windows": {
    "icon": "assets/icons/windows/AppIcon.ico",
  },
}
```

Every value is optional. Names retain their spelling and capitalization for
display. Tokamak derives a lower-case ASCII slug for bundle filenames,
application IDs, and `tokamak.local` hosts, so `My App` becomes `my-app`. If a
platform has no configured name, the Wrangler Worker name is used. The slug is
also used to derive an identifier when no identifier is configured.

Identifiers are used as the Apple bundle identifier and Android application ID.
`TOKAMAK_IDENTIFIER` overrides the configured value, and
`TOKAMAK_ANDROID_IDENTIFIER`, `TOKAMAK_IOS_IDENTIFIER`,
`TOKAMAK_MACOS_IDENTIFIER`, or `TOKAMAK_WINDOWS_IDENTIFIER` override it for
one platform. iOS simulators use the iOS variable.

Icons are platform-specific formats: an Apple Icon Composer `.icon` package
for iOS and macOS, a `res` directory for Android, and an `.ico` file for
Windows. A top-level `icon` therefore only suits platforms that share a format,
so pair a default `.icon` package with `android` and `windows` overrides.
Relative paths are resolved from the directory of the file that names them.

A configuration file may `include` one other configuration file, by absolute
path or relative to the including file. The including file is deep-merged
onto the included one: each key overwrites the same key in the included file,
platform objects merge key by key, and `null` removes an included value. Only
the file tokamak loads may include: an `include` inside the included file is
ignored with a warning. This supports layouts such as a
shared `tokamak.jsonc` with a `tokamak.dev.jsonc` beside it:

```jsonc
// tokamak.dev.jsonc
{
  "include": "../tokamak.jsonc",
  "name": "My Test App",
}
```

The `version` value is optional in the configuration, but is required for
`tok build`. Set it in the configuration or with `TOKAMAK_VERSION`; the
environment variable takes precedence. Development builds keep their existing
native default when no version is supplied. Apple builds use the value for
`CFBundleShortVersionString` and, by default, `CFBundleVersion`; Android uses it
as `versionName`.

Apple platform packs accept an optional per-build number through platform-pack
variables. Use `ios-build-number` for iOS (including the simulator) and
`macos-build-number` for macOS:

```sh
tok build ios --set ios-build-number=5
TOKAMAK_MACOS_BUILD_NUMBER=7 tok build macos
```

These map to `TOKAMAK_IOS_BUILD_NUMBER` and `TOKAMAK_MACOS_BUILD_NUMBER`.
Values must contain one to three period-separated integers. If omitted, the
platform pack continues to use the app version as the Apple build number.

Apple platform packs also accept an optional user-provided application plist.
Set `ios-plist` for iOS devices and simulators, or `macos-plist` for macOS:

```sh
tok build ios --set ios-plist=native/Info.plist
TOKAMAK_MACOS_PLIST=native/Info.plist tok build macos
```

The corresponding environment variables are `TOKAMAK_IOS_PLIST` and
`TOKAMAK_MACOS_PLIST`. Relative paths are resolved from the project directory.
The file may be XML or binary and must have a dictionary at its root. It is
optional; when present, Tokamak layers its generated application values first,
then icon values, plugin values, and finally the user plist. User values
therefore take precedence over all other values, including the SDK, platform,
and Xcode provenance keys Apple platform packs generate from the active
toolchain. Values not supplied by the user are preserved.

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

`--set` may be repeated for multiple platform-pack variables. A value such as
`ios-team-id=YOUR_TEAM_ID` is passed to the iOS platform pack as
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
the two choices. The Apple platform pack owns the signing and provisioning
selection; `tok` passes platform-pack variables through unchanged.

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

## Native plugins

Native capabilities are provided by npm packages such as
`@tokamakdev/plugin-location`. Add them to your project's `dependencies`:

```sh
npm install @tokamakdev/plugin-location
```

`tok build` and `tok dev` include the native code of every plugin listed in
`dependencies`; plugins listed only in `devDependencies` are not included. Call
plugins from browser code. Each plugin's README describes its API.

## Example

[The Astro example](examples/astro) exercises server rendering, static assets,
navigation, WebSockets, and the native location plugin.

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
- CMake, Clang and libclang, and the native C/C++ toolchain for your host.

Platform-pack work additionally requires:

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

### Build a platform pack

Install the Rust target, then build the runtime, native shell, and runtime tools
for one target. Apple platform packs also need both macOS host targets because the
pack includes a universal host-side signing tool:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo run -p xtask -- platform-pack --target macos-arm64
```

Platform packs are written to `target/tokamak-platform-packs/<target>`. Run
`cargo run -p tokamak-cli -- targets` to list supported targets.

### Publish to npm

Merges to `main` publish every npm package with the npm `latest` dist-tag:

- the plugins as `<plugin version>-beta.<run>`;
- `@tokamakdev/tok`, its `@tokamakdev/tok-<host>` binary packages, and the
  `@tokamakdev/platform-<target>` platform packs as
  `<Cargo version>-beta.<run>`. `scripts/package-cli-npm.sh` packages them from
  the release build's CLI and platform-pack archives.

npm Trusted Publishing must be enabled separately for each package for GitHub
user `mantty`, repository `tokamak`, and workflow filename `build.yaml`. A
package must exist before Trusted Publishing can be enabled, so its first
version is published locally.

To package the plugins locally:

```sh
pnpm --dir plugins --filter '@tokamakdev/*' exec pnpm pack --pack-destination "$PWD/artifacts"
```

The shared plugin transport package must be published before a plugin that
depends on it. To publish the first plugin packages locally:

```sh
cd plugins/core
npm publish --access public
cd ../location
npm publish --access public
```

A new CLI or platform-pack package fails to publish from its first
`Build and Pre-Release` run. Publish that run's packages locally, enable
Trusted Publishing for each new package, then re-run the failed job. The
run's artifacts are kept for one day. For the platform packs:

```sh
gh run download RUN_ID --repo mantty/tokamak --name tokamak-npm-platform-packs --dir artifacts
for package in ./artifacts/tokamakdev-platform-*.tgz; do
  npm publish "$package" --access public --tag latest
done
gh run rerun RUN_ID --repo mantty/tokamak --failed
```

### Build the example against local sources

After building a platform pack:

```sh
pnpm --dir examples/astro run build
cargo run -p tokamak-cli -- build macos \
  --project examples/astro \
  --wrangler examples/astro/dist/server/wrangler.json \
  --platform-pack target/tokamak-platform-packs/macos-arm64 \
  --skip-project-build
```

Use the local CLI in development mode with:

```sh
cargo run -p tokamak-cli -- dev macos \
  --project examples/astro \
  --platform-pack target/tokamak-platform-packs/macos-arm64 \
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
pnpm --dir examples/astro install --frozen-lockfile
pnpm --dir examples/astro run build
cargo fmt --all --check
cargo test -p tokamak --features native
node --test tokamak/tests/quickjs_runtime/runtime.test.mjs
cargo test -p tokamak-cli --lib --bin tok --test cli --test platform_pack
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
prebuilt runtime from the platform pack. Static imports, dynamic imports, and
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
