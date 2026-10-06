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
write `tok`. `tok version` prints the installed version.

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

Apps add `@tokamakdev/tok` for the Vite plugin. The installer script is
another way to get `tok` and every platform pack, for your user account. On
macOS, Linux, or WSL:

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
| Android | Android SDK 35 and NDK, Java 17, Gradle, and Bash |
| Windows | 64-bit Windows with Visual Studio or Visual Studio Build Tools, with the "Desktop development with C++" workload |

## AI assistant plugin

This repository includes a repo-installable, skill-only plugin for Codex and
Claude Code. See [the plugin README](ai/plugin/README.md) for installation and
testing instructions.

## Build an application

We do not (currently) support all Cloudflare bindings to additional services they offer, but by and large a basic fullstack web-app written for Cloudflare workers is all you need to build a native app.

tokamak packages the Worker that Vite and Cloudflare's Vite plugin
(`@cloudflare/vite-plugin`) build for Cloudflare. Frameworks whose Worker that
plugin builds are supported: React Router, TanStack Start, Astro 6 and later,
RedwoodSDK, Vike, Next.js through vinext, React and Vue apps, and plain
Workers, including Hono, with a `vite.config.ts`. SvelteKit, Nuxt, Analog,
Solid (Nitro), Qwik, Angular and Next.js through OpenNext are not supported.

Add tokamak's Vite plugin next to Cloudflare's:

```ts
// vite.config.ts
import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";
import { defineConfig } from "vite";

export default defineConfig({ plugins: [cloudflare(), tokamak()] });
```

In Astro, add `tokamak()` to `vite.plugins` in `astro.config.mjs`. The plugin
does nothing unless `tok` runs the build or development command, so builds for
Cloudflare and the web are unchanged.

`tok build` runs the project's build command with the plugin active: the
command given with `--build` or `TOKAMAK_BUILD`, or else the `package.json`
build script with pnpm, Yarn, or npm based on the project's lockfile. It then
packages the Worker the build generated, which it finds as Wrangler does,
through `.wrangler/deploy/config.json`, and writes the native bundle under
`build/<platform>`.

```sh
tok build macos --project ./my-app
```

`--skip-project-build` packages the output of the previous `tok build` without
building the project again. The generated configuration is the only Wrangler
configuration `tok build` reads, so it takes no `--wrangler` or `--env` option.

Platforms are `android`, `ios`, `ios-simulator`, `macos`, and `windows`.
Multiple platforms can be comma-separated, for example `macos,android`.

### Wrangler environments and build reuse

The build chooses a Wrangler environment as it does for Cloudflare: with
`CLOUDFLARE_ENV` set, Cloudflare's plugin applies that environment to the
configuration it generates, including its `vars`. Named vars do not inherit
top-level vars. Values can be strings or JSON.

```jsonc
{
  "name": "my-app",
  "main": "src/index.ts",
  "vars": { "API_URL": "https://api.example.com" },
  "env": {
    "test": { "vars": { "API_URL": "https://test-api.example.com", "OPTIONS": { "preview": true } } },
    "production": { "vars": { "API_URL": "https://api.example.com" } }
  }
}
```

```sh
CLOUDFLARE_ENV=test tok build ios
CLOUDFLARE_ENV=production tok build ios
```

Never put secrets in packaged `vars`.

Built apps also set `TOKAMAK_RUNTIME` to `"true"` in the Worker's `env` and
`process.env`. Plugin helpers use it to serve the runtime's calls to endpoints
under `/tokamak/`.

`build/` holds output and reusable build data; `--build-dir PATH` moves both.
An empty or stale cache builds normally. Changing only vars on Apple updates
the bundled environment file and re-signs without recompiling the shell.
Changing only the Apple build number rewrites the plist and re-signs likewise.

GitHub Actions: use these steps in both a merge build and a later promotion
workflow on the default branch. Promotion takes the tested commit as a
`workflow_dispatch` input named `sha` and sets `CLOUDFLARE_ENV` to `production`
in the final step.

```yaml
- uses: actions/checkout@v6
  with:
    ref: ${{ inputs.sha || github.sha }}
- uses: actions/cache@v4
  with:
    path: build/
    key: tokamak-ios-${{ runner.os }}-${{ inputs.sha || github.sha }}
- run: tok build ios
  env:
    CLOUDFLARE_ENV: test
```

Install `tok` and project dependencies before the build step; upload the signed
output separately.

### Worker modules

tokamak packages the Worker's modules as `wrangler deploy` does for the
configuration the build generated: the Worker's main module and the files in
its directory that the configuration's module rules, then Wrangler's default
rules, match. ES modules are compiled to bytecode. As on Cloudflare, `Text`
modules (`.txt`, `.html` and `.sql` by default) import as strings and `Data`
modules (`.bin`) as `ArrayBuffer`s; both are also files under `/bundle` for
`node:fs`. A `CompiledWasm` (`.wasm`) or `CommonJS` module fails the build:
the runtime has no WebAssembly and loads only ES modules.

### Storage

A packaged app's Worker gets KV, D1 and R2 bindings backed by storage on the
device. Declare them in the Wrangler configuration as for Cloudflare; the
Worker uses the same APIs, so the code deploys to Cloudflare unchanged. The
data stays on the device and is not synced with the Cloudflare resources the
bindings name. `tok build` warns about other bindings, such as Durable Objects
or Queues, which the packaged app does not provide.

```jsonc
{
  "kv_namespaces": [{ "binding": "SETTINGS", "id": "0f2ac74b498b48028cb68387c421e279" }],
  "d1_databases": [{ "binding": "DB", "database_name": "app", "database_id": "…" }],
  "r2_buckets": [{ "binding": "FILES", "bucket_name": "app-files" }]
}
```

| Binding | Worker API | Device store |
| --- | --- | --- |
| `kv_namespaces` | `KVNamespace` | A SQLite database per namespace |
| `d1_databases` | `D1Database` | A SQLite database per database |
| `r2_buckets` | `R2Bucket` | A file per object, with metadata in a SQLite database per bucket |

- A binding's store is created empty the first time the Worker uses it. The
  store is identified by the resource the binding names, `id` for KV,
  `database_id` for D1 and `bucket_name` for R2, or by the binding name when
  there is none. `tok build`
  rejects an identifier that is not a safe file name.
- When the app starts, it deletes the stores no binding names, so removing a
  binding, or pointing it at another resource, removes its data from the device.
- A write whose promise resolves is on disk, and reads see every completed
  write. Requests use a store concurrently, and storage work runs off the
  JavaScript thread.
- Each item's limits are Cloudflare's, so an item that fits on the device fits
  on Cloudflare. Account limits do not apply.
- `tok build` links storage, and SQLite with it, into the app only when the
  Wrangler configuration declares a storage binding. An app without one is
  about 1 MB smaller.

#### KV

`get` (with `text`, `json`, `arrayBuffer` and `stream`, and for up to 100 keys
at once), `getWithMetadata`, `put` (with strings, `ArrayBuffer`s, views and
`ReadableStream`s, `expiration`, `expirationTtl` and `metadata`), `delete` and
`list` behave as local KV does, including their errors. `list` returns keys in
order. An expired key reads as missing and is left out of `list`. `cacheTtl` is
accepted and has no effect.

#### D1

D1 behaves as local D1 does: the same SQLite version and features, the same
refusals (such as `BEGIN`, `ATTACH`, temporary tables and names beginning
`_cf_`), and the same results, errors and metadata. `batch()` runs in one
transaction and `dump()` rejects.

`tok build` packages each database's migrations, found as
`wrangler d1 migrations apply` finds them: `migrations_dir` (default
`migrations`, relative to the Wrangler configuration that declares it) and
`migrations_pattern`. The first query in each app launch applies the migrations
not yet recorded in `migrations_table` (default `d1_migrations`), in order, each
in its own transaction. A failed migration rolls back, and queries on its
binding reject with its error until a later launch applies it.

#### R2

`head`, `get` (with `range` and `onlyIf`, as options or headers), `put`, `delete`
(of one key or many), `list` (with `prefix`, `delimiter`, `cursor`,
`startAfter`, `limit` and `include`) and multipart uploads behave as local R2
does, including their errors. Objects carry R2's etags, checksums and HTTP
metadata, and bodies stream from and to disk. `put` verifies a checksum it is
given, and a failed `onlyIf` returns `null` from `put` and the object without
its body from `get`.

Three behaviours follow Cloudflare where local R2 differs: objects have a
`storageClass`, `Standard` unless `put` sets one; `list` returns
`httpMetadata` and `customMetadata` only when `include` asks for them; and
metadata limits apply to multipart uploads. Customer-provided encryption keys
(`ssecKey`) are rejected, since device storage is encrypted by the operating
system.

As on Cloudflare, a `ReadableStream` written to R2 must have a known length:
a request or response body, or the readable side of a `FixedLengthStream`.

A replaced object reads as either its old or its new version, never a mixture.
Parts of multipart uploads in progress are kept with the runtime's temporary
state, outside backups. If the system removes a part, `complete` fails with
R2's missing-part error.

#### Where data lives

Stores are in the app's private data directory:

| Platform | Directory |
| --- | --- |
| iOS, macOS | `Application Support/<bundle identifier>/tokamak/storage` |
| Android | The app's files directory, `tokamak/storage` |
| Windows | `%LOCALAPPDATA%\<app>\tokamak\storage` |

Uninstalling the app removes them. Device backups include them: iOS and macOS
back up Application Support, and Android's Auto Backup includes the files
directory unless the app's `android.manifest` file opts out. Auto Backup stops
backing up an app whose data exceeds 25 MB.

`tok dev` runs the Worker in the framework's development server, where Wrangler
provides these bindings from the project's `.wrangler/state`, so development
data stays on the development machine.

### Configuration file

The app's tokamak configuration is the `config` export of `src/tokamak.ts`, or
`src/tokamak.js`. The file is optional. Use `-c` or `--config` to name another
file.

```ts
// src/tokamak.ts
import type { Config } from "@tokamakdev/tok";

export const config = {
  // Defaults for every platform
  name: "My App",
  identifier: "com.example.myapp",
  icon: "../assets/icons/AppIcon.icon",
  version: "1.0.0",

  // Platform overrides and platform-pack settings
  ios: {
    name: "Myapp Pro",
    identifier: "com.example.myapp.ios",
    plist: "../native/Info.plist",
  },
  android: {
    icon: "../assets/icons/android",
  },
  windows: {
    icon: "../assets/icons/windows/AppIcon.ico",
  },
} satisfies Config;
```

The top-level keys are `name`, `identifier`, `icon`, and `version`; other
top-level keys are rejected. They are defaults for every platform. A platform
object (`android`, `ios`, `macos`, `windows`) overrides `name`, `identifier`,
or `icon` for that platform, and its other keys are settings for that
platform's pack. `version` is top-level only. `ios` covers iOS devices and
simulators. Paths are relative to the configuration file.

The plugin reads the file with Vite, using the app's aliases, when `tok build`
or `tok dev` runs, so the file can import other modules and read
`process.env`. A shared base and per-environment variants are ordinary imports
and conditions, so there is no `include` key, and no `tokamak.jsonc`:

```ts
import type { Config } from "@tokamakdev/tok";
import { base } from "./tokamak.base";

export const config = {
  ...base,
  name: process.env.APP_NAME ?? "My App",
} satisfies Config;
```

In tokamak builds and development the file is also part of the Worker: its
top-level code runs whenever the Worker is evaluated, including during Astro's
prerendering at build time, and when the configuration is read. Keep that code
cheap and safe to run in each of those places. The Worker's copy of the file
has no `config` export, so declare it as `export const config = ...` in a
statement of its own. It keeps `config` as a local constant while the file's
other code mentions `config`, and leaves it out otherwise. Builds for
Cloudflare and the web do not include the file.

Every value is optional. Names retain their spelling and capitalization for
display. Tokamak derives a lower-case ASCII slug for bundle filenames,
application IDs, and `tokamak.local` hosts, so `My App` becomes `my-app`. If a
platform has no configured name, the Wrangler Worker name is used as both name
and slug, so it must already be a slug: lowercase letters and digits joined by
single hyphens, at most 63 characters. Set a name for Worker names that are
not. The slug is also used to derive an identifier when no identifier is
configured.
Identifiers are used as the Apple bundle identifier and Android application ID.

Icons are platform-specific formats: an Apple Icon Composer `.icon` package
for iOS and macOS, a `res` directory for Android, and an `.ico` file for
Windows. A top-level `icon` therefore only suits platforms that share a format,
so pair a default `.icon` package with `android` and `windows` overrides.

The `version` value is required for `tok build`. Development builds keep their
existing native default when no version is supplied. Apple builds use it for
`CFBundleShortVersionString` and, by default, `CFBundleVersion`; Android uses
it as `versionName`.

### Settings

Every setting can be given as a command-line option, an environment variable,
or a configuration key. A setting has the same name in each:

| Setting | Option | Environment variable | Configuration |
|---|---|---|---|
| Top-level | `--version` | `TOKAMAK_VERSION` | `version` |
| Platform | `--ios-plist` | `TOKAMAK_IOS_PLIST` | `ios.plist` |

An option takes precedence over an environment variable, which takes precedence
over the configuration. Within each source, a platform's own value takes
precedence over a top-level one, so `--ios-identifier` overrides
`--identifier`.

- **Top-level settings:** `name`, `identifier`, `icon`, and `version`. The
  project build command is an option and environment variable only:
  `--build` or `TOKAMAK_BUILD`, for `tok build`.
- **Platform settings:** `name`, `identifier`, and `icon` for each platform,
  which tokamak validates, plus the settings that platform's pack declares.
  `tok build <platform> --help` and `tok dev <platform> --help` list them. A
  setting the pack does not declare is an error for the platform being built;
  settings for other platforms are ignored.
- **Values:** option and environment values are strings. Configuration values
  may be strings, numbers, or booleans.
- **Relative paths:** paths in options and environment variables are relative
  to the current directory. Paths in a configuration file are relative to that
  file's directory.

The platform packs accept these settings:

| Setting | Platforms | Value |
|---|---|---|
| `build-number` | `ios`, `macos` | `CFBundleVersion`: one to three period-separated integers; defaults to the app version |
| `plist` | `ios`, `macos` | Application plist layered over generated values |
| `entitlements` | `ios`, `macos` | Entitlements plist layered over the provisioning profile's entitlements |
| `team-id` | `ios`, `macos` | Apple Development team for signing |
| `signing-identity` | `ios` | Identity SHA-1 for manual signing |
| `provisioning-profile` | `ios` | Provisioning profile for manual signing |
| `manifest` | `android` | Partial `AndroidManifest.xml` merged over the generated manifest |
| `keystore` | `android` | Keystore that signs release builds |
| `keystore-password` | `android` | Password of the keystore |
| `key-alias` | `android` | Alias of the signing key in the keystore |
| `key-password` | `android` | Password of the signing key; defaults to the keystore password |

```sh
tok build ios --ios-build-number 5
TOKAMAK_MACOS_PLIST=native/Info.plist tok build macos
```

The application plist may be XML or binary and must have a dictionary at its
root. Tokamak layers its generated application values first, then icon values,
each plugin's plist, and finally the application plist:

- A plugin's plist merges onto the layers before it. Dictionaries merge by key,
  and arrays merge as a union, so two plugins can each add a
  `UIBackgroundModes` value. A different value at the same key path fails the
  build, naming the key path and both sources, unless the application plist
  sets that key path.
- The application plist overlays everything: dictionaries merge, and any other
  value, including an array, replaces. Its values therefore take precedence
  over all other values, including the SDK, platform, and Xcode provenance keys
  Apple platform packs generate from the active toolchain.

The entitlements file is a plist with a dictionary root. Signing uses the
provisioning profile's entitlements overlaid with the file's, with the plist
overlay rules. Each declared entitlement must be one the profile permits: the
same value, `*`, or a prefix wildcard such as `TEAMID.*`. Automatic signing
selects a profile that permits every declared entitlement and asks Xcode to
provision one when none does, which enables the matching capabilities on the
App ID. Manual signing fails when the profile does not permit one. The
`aps-environment` entitlements take the profile's value. iOS Simulator builds
embed the declared entitlements in the executable. macOS builds without a team
are signed with the file as given, and fail when it declares an entitlement
only a provisioning profile can authorise.

The Android manifest is a partial `AndroidManifest.xml`:

```xml
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">
  <uses-permission android:name="android.permission.CAMERA" />
  <application android:allowBackup="false" />
</manifest>
```

The Android Gradle Plugin's manifest merger combines it with the generated
manifest, which includes plugin permissions, and each plugin's own manifest.
The application manifest has the highest priority:

- Elements are combined by key, for example `<uses-permission>` by
  `android:name`. An element declared in both files appears once.
- An attribute the generated manifest does not set is added.
- An attribute the generated manifest sets to a different value fails the
  build, and the merger's error names the attribute. Adding
  `tools:replace="android:<attribute>"` to the element in the application
  manifest replaces the generated value.
- `tools:node="remove"` on an element in the application manifest removes it
  from the merged manifest, including a permission a plugin declares.

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
the package into the platform bundle. Invalid paths or platform formats fail
the native build.

### Signing

Physical iOS builds use automatic development signing by default. Tokamak
selects an available Apple Development team automatically. If the choice is
ambiguous, it lists the team names from the signing certificates and their
IDs. Choose the team that owns your app and set it as `ios.team-id`, with
`TOKAMAK_IOS_TEAM_ID`, or for a single command:

```sh
tok dev DEVICE_ID --ios-team-id YOUR_TEAM_ID -- pnpm dev
tok build ios --ios-team-id YOUR_TEAM_ID
```

Tokamak selects the signing identity and provisioning profile for that team,
asking Xcode to provision the app when needed. No manual signing settings
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

Use the identity's SHA-1 and the profile's path:

```sh
tok build ios \
  --ios-signing-identity IDENTITY_SHA1 \
  --ios-provisioning-profile /path/to/profile.mobileprovision
```

An explicit team and the manual pair are mutually exclusive. Providing both
produces an error explaining the two choices. The Apple platform pack owns the
signing and provisioning selection.

Both `tok dev` and `tok build ios` use these settings; the latter provisions
for a generic iOS device and does not require a device ID. iOS Simulator
builds do not require provisioning.

macOS builds are ad-hoc signed unless `macos.team-id` is set:

```sh
tok build macos --macos-team-id YOUR_TEAM_ID
```

Tokamak then signs with an Apple Development identity and a macOS development
profile for that team that includes this Mac, asking Xcode to register the Mac
and provision the app when no installed profile matches. Team-signed builds can
use the data protection keychain, which `@tokamakdev/plugin-secure-storage`
requires on macOS. They run only on Macs registered to the team. Distribution
signing (Developer ID, the hardened runtime, and notarisation) is not covered.

## Development

tokamak supports dev mode with HMR via Vite and Cloudflare's Vite plugin, proxying to a device for native capabilities.
Dev mode is significantly less performant than a real build, but provides an excellent local dev loop.

To list available local devices, simulators, and emulators for dev mode:

```sh
tok devices
```

Pass a device selector and the framework's development command to `tok dev`:

```sh
tok dev macos --project ./my-app -- pnpm dev
```

The tokamak Vite plugin reports the development server's address, the
configuration, and the Worker name to `tok dev`, which waits up to a minute for
them, then builds the development app and proxies it to that address, so
`tok dev` takes no `--server` option. The plugin reads the Worker name with
Wrangler from `wrangler.jsonc`, `wrangler.json` or `wrangler.toml` in the Vite
root, in the `CLOUDFLARE_ENV` environment. Restart `tok dev` after changing
native settings.

## Native plugins

Native capabilities are provided by npm packages: `@tokamakdev/plugin-location`,
`@tokamakdev/plugin-secure-storage`, `@tokamakdev/plugin-local-authentication`
and `@tokamakdev/plugin-notifications`. Add them to your project's
`dependencies`:

```sh
npm install @tokamakdev/plugin-location
```

`tok build` and `tok dev` include the native code of every plugin listed in
`dependencies`, `devDependencies`, or `peerDependencies`. tokamak finds each
package as Node does, in the nearest `node_modules` of the project or a parent
directory, so plugins installed at a workspace root are included. Call plugins
from browser code. Each plugin's README describes its API.

A plugin can also run Worker code: native builds post an event the plugin
receives, such as a data-only push notification, to an endpoint under
`/tokamak/` that the Worker's `fetch` serves with the plugin's helper. In
`tok dev` the post reaches the development server. The helper responds 404 on
Cloudflare, so the same Worker deploys unchanged.

`tok build android` produces a release APK, which R8 shrinks by removing
unused code; it keeps every class and member name. The keystore settings sign
the APK; without a keystore, the debug key signs it, so it installs for testing
but cannot be published. Set the passwords through the
`TOKAMAK_ANDROID_KEYSTORE_PASSWORD` and `TOKAMAK_ANDROID_KEY_PASSWORD`
environment variables rather than the configuration file. `tok dev` builds debug APKs.

Android builds run lint's `NewApi` check over the shell and plugin sources. A
call to an API newer than the app's minimum SDK fails the build, naming the
file, line and required API level, unless a `Build.VERSION.SDK_INT` check
guards it.

### Camera and microphone

Pages use `getUserMedia`. The shells grant camera and microphone requests from
the app origin once the app has declared them, and the operating system asks
the user on first use. Other origins keep the WebView's default behavior, and a
refused request rejects with `NotAllowedError`.

| Platform | Declaration |
|---|---|
| iOS | `NSCameraUsageDescription` and `NSMicrophoneUsageDescription` in the `ios.plist` file |
| macOS | The same keys in the `macos.plist` file |
| Android | `android.permission.CAMERA` and `android.permission.RECORD_AUDIO` in the `android.manifest` file |
| Windows | None; the shell asks the user, as it does for location |

## Example

[The Astro example](examples/astro) exercises server rendering, static assets,
navigation, WebSockets, and the native location plugin. It installs
`@tokamakdev/tok` from this repository; its README lists the setup.

```sh
tok dev macos --project examples/astro -- pnpm dev
tok build macos --project examples/astro
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

Install the JavaScript dependencies once after cloning. The Astro example
installs `@tokamakdev/tok` from `tokamak-cli/npm`, so build that package first:

```sh
npm ci --prefix tokamak-cli/npm
npm run build --prefix tokamak-cli/npm
pnpm --dir plugins install --frozen-lockfile
pnpm --dir examples/astro install --frozen-lockfile
```

### Build the CLI

```sh
cargo build --release -p tokamak-cli
```

The executable is written to `target/release/tok` (`tok.exe` on Windows).

### Build a platform pack

Install the Rust target, then build the runtime and native shell for one target. Apple platform packs also need both macOS host targets because the
pack includes a universal host-side signing tool:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo run -p xtask -- platform-pack --target macos-arm64
```

Platform packs are written to `target/tokamak-platform-packs/<target>`. Run
`cargo run -p tokamak-cli -- targets` to list supported targets.

Each pack declares the settings it accepts in
`platforms/<pack>/build/variables.json`, keyed by platform. A setting has a
`kind` (`string`, or `path` for a path tokamak makes absolute) and a one-line
`description` for `--help`. The build writes the target platform's declarations
into `platform-pack.json`, and `tok` passes each setting to the entrypoint as
`TOKAMAK_<PLATFORM>_<KEY>`.

### SQLite

The runtime's SQLite is generated into the vendored `libsqlite3-sys` crate with
the version, workerd patches and options that D1 uses.
`libsqlite3-sys/TOKAMAK.md` describes them, and
`cargo run -p xtask -- sqlite` regenerates the source.

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

A new plugin, CLI or platform-pack package fails to publish from its first
`Build and Pre-Release` run. Publish that run's new packages locally, enable
Trusted Publishing for each, then re-run the failed job. The run's artifacts
are kept for one day. For the platform packs:

```sh
gh run download RUN_ID --repo mantty/tokamak --name tokamak-npm-platform-packs --dir artifacts
for package in ./artifacts/tokamakdev-platform-*.tgz; do
  npm publish "$package" --access public --tag latest
done
gh run rerun RUN_ID --repo mantty/tokamak --failed
```

For a plugin, download the `tokamak-plugins` artifact instead and publish the
new plugin's tarball, for example
`./artifacts/tokamakdev-plugin-notifications-<version>.tgz`.

### Build the example against local sources

After building a platform pack:

```sh
cargo run -p tokamak-cli -- build macos \
  --project examples/astro \
  --platform-pack target/tokamak-platform-packs/macos-arm64
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
npm ci --prefix tokamak-cli/npm
npm run build --prefix tokamak-cli/npm
npm test --prefix tokamak-cli/npm
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
Storage bindings share their stores between requests.
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
