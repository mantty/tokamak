# tokamak as a library

Status: implemented for macOS, iOS, Android, and Windows. Linux remains a
future platform.

First of three: this, then `tokamak-api-testing.md`, then `tokamak-plugins.md`.

## Purpose

tokamak used to be the application: it created the application object, window,
WebView, and event loop. That inverted. A per-platform shell is the
application and drives tokamak through a public API.

tokamak is pre-release. Backwards compatibility is not a goal.

## Rust structure

| Directory | Owns |
|---|---|
| `tokamak/src/quickjs` | QuickJS embedding and the gateway |
| `tokamak/src/server.rs`, `tokamak/src/lifecycle_events.rs` | application startup and lifecycle |
| `tokamak/src/certificates.rs`, `tokamak/src/cert_*.rs` | local mTLS certificate material and trust decisions |
| `tokamak/src/packaging.rs`, `tokamak/src/compat.rs`, `tokamak/src/env_vars.rs`, `tokamak/src/asset_manifest.rs`, `tokamak/src/wrangler_config.rs` | the on-disk Worker contract and source preparation |
| `tokamak/src/fs` | the Workers virtual filesystem and native `node:fs` bindings |
| `tokamak/src/streams` | Node and Web Streams, text encoding, and adapters |
| `tokamak/src/network` | Fetch, URL, and WebSocket APIs |
| `tokamak/src/events` | Event and EventEmitter APIs |
| `tokamak/src/globals` | Web, process, and console globals |
| `tokamak/src/builtins` | synthetic builtin entrypoints and registration |
| `tokamak/src/android_jni.rs`, `tokamak/src/apple_ffi.rs` | target-specific runtime bridges |
| `tokamak-cli` | user-facing native app packaging |
| `platforms/apple/source` | Swift shell, C ABI header, and module map |
| `platforms/apple/build` | Apple target-pack recipe and app build entrypoint |
| `platforms/android/source` | Kotlin shell sources |
| `platforms/android/build` | Android target-pack recipe and app build entrypoint |
| `platforms/windows/source` | Rust Windows application shell |
| `platforms/windows/build` | Windows target-pack recipe and app build entrypoint |
| `tools/xtask` | maintainer-only target-pack generation |
| `tokamak-cli/src/target_pack.rs` | target-pack manifest format and target metadata |
| `platforms/apple/signing` | Apple signing discovery and target-pack signing tool |

The root-level Worker-package modules own the Worker layout and resolve every
path within it, so neither `tokamak-cli` nor the runtime names a file. `tokamak-cli`
writes the contract and the runtime reads it.

The `tokamak` package contains the shared runtime and its Apple C and Android JNI
bridges. Every platform shell remains under `platforms/`, regardless of its
implementation language. The CLI stages common app inputs and invokes the
uniform target-pack entrypoint. Target packs own platform project generation,
signing, and packaging; target-pack variables pass through as
`TOKAMAK_<PLATFORM>_<NAME>` environment variables. Components inside `tokamak`
depend on each other through narrow interfaces.

## Certificates

- Generates the CA, server certificate, and client certificate.
- Caches them in the app state directory.
- Renews them before expiry, on a background thread.
- Decides TLS auth challenges: which identity to present, and whether a server
  certificate is trusted. The shell wires those decisions into the platform
  callback.

## Runtime

- Starts and stops QuickJS, and owns the runtime handle.
- Builds the runtime config from paths resolved by the root-level Worker-package
  modules and the
  certificates.
- Loads app code, already prepared by `tokamak-cli`.
- Exposes explicit suspend and resume operations for callers that need them.
- Emits lifecycle events — starting, listening, suspended, resumed, failed —
  for shells and, later, plugins.
- Learns the gateway's port when its listener binds. Startup does not poll.

## Shell

The shell owns the process entry point, application object, UI event loop,
window, WebView, and WebView proxy configuration. It starts and stops tokamak,
answers platform callbacks using the certificate component, and recovers the
gateway after mobile foreground transitions when the listener changed ports.

macOS and iOS use the same Swift shell and C runtime ABI. Android uses a Kotlin
shell and JNI runtime ABI. Windows uses the Rust shell under
`platforms/windows`. All target packs have the same boundary: a compiled tokamak
artifact, native shell sources where the platform compiles the shell during app
assembly, or a precompiled shell executable, and the fixed build entrypoint.
The entrypoint receives `build INPUT OUTPUT`, inherits target-pack variables, and
owns the platform-specific build, signing, and packaging work. Developers need
the native platform toolchain to build and sign an app; the Windows target pack
additionally contains the precompiled Rust shell.

Desktop focus, minimization, and occlusion do not suspend the tokamak runtime.
Android and iOS likewise leave the runtime available to the operating system
while backgrounded; on foreground, their shells probe the gateway and update
the WebView proxy when its port changed. A changed Android proxy reloads the
WebView. The operating system may still freeze or terminate a mobile process.

## Done when

Every shell runs tokamak through the public API, the library creates no
application object or event loop, and every app build links a precompiled
runtime library without invoking Cargo.
