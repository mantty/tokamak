# tokamak plugins

Status: frontend plugin foundation implemented

## Purpose

Give app code access to device capabilities through npm packages that retain a
normal web implementation.

## Package contract

An app declares a plugin by adding its package to `dependencies`. No separate
tokamak configuration is required.

A frontend plugin package contains:

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

`tokamak-plugin.json` declares the plugin ID, kind, platform sources, native class,
linked Apple frameworks, Info.plist values, and Android permissions. tokamak
discovers manifests from the app's direct dependencies.

A platform omitted from the manifest has no native implementation. Calls made
without an implementation throw `NotSupportedError`.

## Native build

The tokamak runtime remains prebuilt. `tok build` stages plugin metadata and
sources, then the target-pack entrypoint compiles only the native shell and
plugin sources:

- macOS, iOS, and iOS Simulator compile Swift sources into the application
  executable and link declared system frameworks.
- Android compiles Kotlin sources into the application and merges declared
  permissions into its manifest.

tokamak generates one registry per application, so plugins do not need manual
shell registration.

## Frontend calls

Native calls use the WebView's process bridge rather than TCP:

- Apple uses `WKScriptMessageHandler`.
- Android uses `WebViewCompat.addWebMessageListener`.

Both bridges accept messages only from the main frame at the exact
`https://<app>.tokamak.local` origin. Calls return promises. Subscriptions return a
function that stops native updates.

Values crossing the bridge are JSON-compatible primitives, objects, and
arrays. Native errors retain their DOM exception name and message.
Each page load has a distinct bridge session. Navigation cancels native
subscriptions and late responses from the previous page are ignored.

## First plugin

`@tokamakdev/plugin-location` supports:

- `getCurrentPosition()`
- `watchPosition(next, error)`, returning a stop function

Web uses `navigator.geolocation`. macOS, iOS, and iOS Simulator use
`CoreLocation`. Android uses `LocationManager`. Windows uses WebView2's
location implementation, with consent restricted to the app origin.

## Deferred

- Backend plugins and Bare worker bindings.
- Native plugin artifact manifests for languages other than the shell's Swift
  or Kotlin.
- Entitlements and platform metadata beyond string Info.plist values, Apple
  frameworks, and Android permissions.
- Plugin API compatibility declarations.
- Compiled ESM and declaration files for npm releases.
- Background capability dispatch with no page or worker request in flight.
