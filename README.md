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

The supported values are `name` and `icons`:

```jsonc
{
  "name": {
    "default": "My App",
    "ios": "Myapp Pro",
  },
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
used for both iOS devices and iOS simulators. Names are normalized to a
lower-case ASCII slug for bundle filenames, application IDs, and
`tokamak.local` hosts, so `My App` becomes `my-app`. If `name` is absent, the
Wrangler Worker name is used.

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
selects an available Apple Development team automatically; set
`TOKAMAK_IOS_TEAM_ID` to choose a team explicitly. For a fully manual signing
selection, set both `TOKAMAK_IOS_SIGNING_IDENTITY` and
`TOKAMAK_IOS_PROVISIONING_PROFILE`. These variables are used by both `tok dev`
and `tok build ios`; the latter provisions for a generic iOS device and does
not require a device ID. iOS Simulator builds do not require provisioning.

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
  --skip-web-build
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
for one target:

```sh
rustup target add aarch64-apple-darwin
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
  --skip-web-build
```

Use the local CLI in development mode with:

```sh
cargo run -p tokamak-cli -- dev macos \
  --project examples/astro \
  --target-pack target/tokamak-target-packs/macos-arm64 \
  -- pnpm dev
```

### Checks

Run the common checks before submitting a change:

```sh
cargo fmt --all --check
cargo test -p tokamak --features native
cargo test -p tokamak-cli --lib --bin tok --test cli --test target_pack
cargo test -p xtask
pnpm --dir plugins lint:ts
pnpm --dir plugins test:ts
```

Platform-specific lint and build-test commands are kept in
[the Checks workflow](.github/workflows/test.yaml).
