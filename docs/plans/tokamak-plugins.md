# tokamak plugins

Status: plugin foundation implemented

## Purpose

Give app code access to device capabilities through npm packages that retain a
normal web implementation.

## Package contract

An app declares a plugin by adding its package to `dependencies`,
`devDependencies`, or `peerDependencies`. No separate tokamak configuration is
required.

A plugin package contains:

```text
<package>/
  tokamak-plugin.json
  src/
  web/
  apple/
  android/
```

The TypeScript entrypoint is the API imported by app code. It calls the web
implementation in a browser and the native implementation when tokamak provides
its frontend bridge.

`tokamak-plugin.json` declares the plugin ID and a section per platform
namespace: `android`, `ios` (devices and the iOS Simulator), `macos` and
`windows`. Each platform pack declares the keys its section takes in
`build/plugin-keys.json` and validates their values. Apple packs take the
native `class`, Swift `sources` and an Info.plist file
(`"plist": "apple/Info.plist"`). The Android pack takes the `class`, Kotlin
`sources`, an `AndroidManifest.xml` file (`"manifest"`), which declares the
plugin's permissions, and Maven dependencies
(`"dependencies": ["group:artifact:version"]`). The Windows pack takes no
plugins. tokamak discovers manifests from the app's direct dependencies,
resolved from the nearest `node_modules` of the app or a parent directory.

A platform omitted from the manifest has no native implementation. Calls made
without an implementation throw `NotSupportedError`.

A manifest may declare build `settings`, each with a `kind` (`string` or
`path`) and a `description`, as a platform pack's `variables.json` does. Apps
set them under the plugin's ID: `--<id>-<key>`, `TOKAMAK_<ID>_<KEY>` and
`<id>.<key>`, and for one platform `--<id>-<platform>-<key>`,
`TOKAMAK_<ID>_<PLATFORM>_<KEY>` and `<id>.<platform>.<key>`. tok rejects plugin
IDs that start with a platform name, a top-level key of the `tokamak()` options
or the first word of one of its own options or environment variables, and
settings whose options would start with another plugin's longer ID. A manifest
that declares settings names a Node `build` script in the package, which tok
runs for each platform the plugin has a section for, with
`{ platform, settings }` as JSON on stdin. The script validates the settings,
exiting non-zero with its message on stderr to fail the build, and prints a
JSON object of keys to add to the plugin's section, whose files are absolute
paths. The Apple packs take `resources`, files placed in the app bundle's
resources directory, and the Android pack takes `raw`, files added as raw
resources.

## Native build

The tokamak runtime remains prebuilt. `tok build` stages each key of a
plugin's section at `plugins/<id>/<key>` in the build input: a string as a file
holding it, a file as its copy, unread, and a list as a directory of its items
named by index, with a file's name after its index (`sources/0-Plugin.swift`).
A key the pack does not declare, a value of another kind, or a missing file
fails the build before any platform builds. The platform-pack entrypoint then
compiles only the native shell and plugin sources:

- macOS, iOS, and iOS Simulator compile Swift sources into the application
  executable, which links the system frameworks they import. The Apple helper
  merges each plugin's Info.plist by key path.
- Android builds each plugin as a Gradle library module that depends on a
  plugin API module (`TokamakPlugin`, `TokamakHost` and their types) and the
  plugin's declared dependencies. The plugin's manifest merges below the app's.

tokamak generates one registry per application, so plugins do not need manual
shell registration.

## Plugin lifetime

Each platform has one `TokamakHost` per process: `TokamakHost` on Apple
platforms and the `TokamakApplication` on Android. It owns the runtime and the
plugins, which it creates once, before any page loads: during launch on Apple
platforms, and on first use on Android, including when the system starts the
app in the background. A plugin receives the host in its constructor
(`init(host:)` in Swift, a constructor parameter in Kotlin).

- Apple plugins receive app events through optional protocol methods: remote
  notification registration results, and remote notifications with a
  completion to call once.
- Android plugins receive the activity's launch intent and later intents
  through `onIntent`, ask for runtime permissions through
  `host.requestPermissions`, which shows one request at a time, and reach the
  activity through `host.activity`.
- `host.call` posts JSON to the Worker's `/tokamak/<name>` endpoint and
  returns the response body. The runtime attempts a failed post again a second
  later, up to three times, within the timeout the plugin gives, which fits the
  platform's background budget and includes runtime startup.

## Frontend calls

Native calls use the WebView's process bridge rather than TCP:

- Apple uses `WKScriptMessageHandler`.
- Android uses `WebViewCompat.addWebMessageListener`.

Both bridges accept messages only from the main frame at the exact
`https://<app>.tokamak.local` origin. Calls return promises. Subscriptions return a
function that stops native updates.

Values crossing the bridge are JSON-compatible primitives, objects, and
arrays. Native errors retain their DOM exception name and message. Plugins
reply from any thread; the bridges deliver replies on the main thread.
Each page load has a distinct bridge session. Navigation cancels native
subscriptions and late responses from the previous page are ignored.

Plugin Info.plist files merge onto the generated values: dictionaries by key,
arrays as a union. A different value at the same key path fails the build
unless the app's plist sets that key path.

## Plugins

`@tokamakdev/plugin-location` supports:

- `getCurrentPosition()`
- `watchPosition(next, error)`, returning a stop function

Web uses `navigator.geolocation`. macOS, iOS, and iOS Simulator use
`CoreLocation`. Android uses `LocationManager`. Windows uses WebView2's
location implementation, with consent restricted to the app origin.

`@tokamakdev/plugin-secure-storage` stores device-only strings by name, in the
Keychain on Apple platforms and with per-value Android Keystore keys on
Android. `@tokamakdev/plugin-local-authentication` reports and performs device
owner authentication. Neither has a web implementation.

`@tokamakdev/plugin-notifications` shows, schedules and receives notifications
on Apple platforms, Android and the web, and posts data-only push messages to
the Worker's `/tokamak/push` endpoint.

## Deferred

- Worker bindings provided by plugins.
- Native plugin artifact manifests for languages other than the shell's Swift
  or Kotlin.
- Plugin entitlements files.
- Plugin API compatibility declarations.
- Compiled ESM and declaration files for npm releases.
- Windows plugins.
