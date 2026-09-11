---
name: tokamak-development
description: "Build, run, debug, and design applications with tokamak. Use for tokamak projects, tokamak CLI commands, Worker-compatible application code, Wrangler configuration, native and web portability, WebView layout, Cloudflare binding limits, native plugins, mobile backgrounding, connection recovery, local state, or questions about how tokamak differs from Cloudflare Workers."
---

# tokamak development

Apply the following model whenever working on a tokamak application. tokamak is in early alpha; if the installed CLI or current repository conflicts with this skill, treat the installed version and current source as authoritative.

## Model tokamak correctly

- Treat tokamak as a cross-platform application framework, not as a Cloudflare hosting product and not as a frontend-only WebView wrapper.
- Build the UI with ordinary HTML, CSS, and browser JavaScript. Run the server entrypoint with Cloudflare Worker-style request semantics.
- In a packaged native app, run the Worker-compatible server code locally on the user's device inside tokamak's embedded runtime. Package the Worker, static assets, native shell, and declared native plugins into the application.
- Load the UI from a stable secure `https://<app-host>.tokamak.local/` origin. Let the native shell route same-origin HTTP and WebSocket traffic to tokamak's local runtime.
- Remember that “Worker compatible” describes an API and build shape. It does not mean that Cloudflare deploys, hosts, or supplies services to a native app.
- Treat compatibility as one-way: code written within tokamak's supported subset can also target Cloudflare Workers, but arbitrary Cloudflare Worker applications may depend on APIs or bindings tokamak does not provide.
- Use the project's normal framework or Cloudflare tooling for a web deployment. The current tokamak CLI has native targets only; there is no `tok build web` command.

## Distinguish development from packaged execution

### `tok dev`

- Run the framework development command on the development computer.
- Build and launch a native development shell on the selected device, simulator, emulator, or desktop.
- Proxy the shell's secure app origin to the host development server, including WebSocket traffic, so framework HMR can work inside the native shell.
- Expect server-side behavior during `tok dev` to come from the framework's host process. Do not use development mode as proof that a Worker API or Cloudflare binding exists in the packaged tokamak runtime.
- Expect native frontend plugins to remain available through the development shell.

### `tok build`

- Run the project's `package.json` build script unless `--skip-project-build` is explicitly used.
- Bundle the built Worker and assets into each requested native application under `build/<platform>`.
- Run subsequent same-origin requests entirely through the packaged app and its embedded runtime. Do not assume a Node server, Wrangler process, internet connection, or Cloudflare account is present.

## Use the tokamak CLI

Use these current command shapes:

```sh
tok devices
tok dev <device-selector> --project <project> -- <framework-dev-command>
tok build <platform[,platform...]> --project <project>
tok targets
```

- Use `--server http://<host>:<port>` when the framework development server is not at `http://localhost:5173`.
- Use `-w` or `--wrangler <path>` when the relevant Wrangler file is generated outside the project root.
- Use `--config <path>` for the optional Tokamak configuration file; it defaults to the current directory.
- Pass `--skip-project-build` only when the project output already exists and is current.
- Use the current native platform names: `android`, `ios`, `ios-simulator`, `macos`, and `windows`.
- Require a `package.json` build script and a Wrangler configuration with at least `name` and `main` for a packaged build.
- Let tokamak detect pnpm, Yarn, or npm from the project's lockfile when it runs the build.

### Configuration and target-pack variables

- Look for `tokamak.jsonc` in the current directory, then `tokamak.json`; the file is optional. Use `-c` or `--config` to select another file or directory.
- JSONC permits comments and trailing commas; plain JSON is also supported.
- The supported configuration values are `name`, `identifier`, `version`, and `icons`.
- `name` and `identifier` use a required `default` value plus optional platform values. Platform values fall back to `default`; `ios` also applies to `ios-simulator`.
- Display names preserve their spelling and capitalization. Tokamak derives a lower-case slug for filenames, application IDs, and local hosts. If `name` is absent, the Wrangler Worker name is used.
- `identifier` values are used as the Apple bundle identifier and Android application ID. `TOKAMAK_IDENTIFIER` and platform-specific `TOKAMAK_ANDROID_IDENTIFIER`, `TOKAMAK_IOS_IDENTIFIER`, `TOKAMAK_MACOS_IDENTIFIER`, or `TOKAMAK_WINDOWS_IDENTIFIER` override configured identifiers; the iOS value also applies to simulators.
- `version` is optional in configuration but required by `tok build`. `TOKAMAK_VERSION` overrides the configured value. Do not use `TOKAMAK_APP_VERSION`.
- `icons` accepts user-created platform assets: Android `res` directories, Apple `.icon` packages for `ios`/`macos`, and Windows `.ico` files. Missing icons preserve the existing behavior. Icon paths are relative to the configuration file.
- Use repeatable `--set NAME=VALUE` options for target-pack variables. `tokamak` maps `ios-team-id=TEAM` to `TOKAMAK_IOS_TEAM_ID`, for example, and passes target-pack variables through without interpreting platform-specific names.

Examples:

```sh
tok build ios --config ./tokamak.jsonc --set ios-build-number=5
tok build ios --set ios-plist=native/Info.plist
TOKAMAK_MACOS_PLIST=native/Info.plist tok build macos
```

Apple target packs accept optional user-provided XML or binary application
plists through `ios-plist`/`TOKAMAK_IOS_PLIST` and
`macos-plist`/`TOKAMAK_MACOS_PLIST`. Relative paths are resolved from the
project directory. The plist must have a dictionary root. Tokamak layers its
generated values first, then icon values, plugin values, and the user plist
last; user-defined values therefore take precedence over all other values.
Values not supplied by the user are retained.

Apple build numbers are target-pack variables rather than Tokamak config:
`ios-build-number` maps to `TOKAMAK_IOS_BUILD_NUMBER` and
`macos-build-number` maps to `TOKAMAK_MACOS_BUILD_NUMBER`. They are optional
and use the app version by default.

For physical iOS signing, use either `ios-team-id`/`TOKAMAK_IOS_TEAM_ID` for
automatic selection or both `ios-signing-identity`/`TOKAMAK_IOS_SIGNING_IDENTITY`
and `ios-provisioning-profile`/`TOKAMAK_IOS_PROVISIONING_PROFILE` for manual
signing. Do not provide both modes. Use `tok certs` to inspect installed
identities and profiles. `tok build ios` does not need a device ID; simulator
builds do not require provisioning.

## Respect the packaged Worker contract

- Export a default Worker object with a `fetch(request, env, ctx)` handler, directly or through a compatible framework adapter.
- Use standard request and response semantics and same-origin routes between the frontend and packaged Worker.
- Expect a fresh JavaScript runtime and module graph for each packaged HTTP request. Do not use module globals, singleton objects, or in-memory caches as durable state across requests.
- Treat a WebSocket Worker context as lasting only for that WebSocket connection.
- Treat the packaged `node:fs` view as request-scoped: `/bundle` contains read-only packaged files and `/tmp` is fresh for the request. Do not use it for persistent application data.
- Check tokamak's current support before relying on a specific Cloudflare Worker or Node API. Similar syntax is not evidence that every Cloudflare or Node behavior exists.

## Do not assume Cloudflare bindings exist

- Treat Wrangler `vars` containing text or JSON as the supported Worker environment values.
- Treat Wrangler static assets as files packaged and routed by tokamak before Worker dispatch.
- Do not treat `env.ASSETS` as a full Cloudflare asset service; tokamak's static asset router is the supported path and `env.ASSETS.fetch()` is not currently implemented as an asset lookup.
- Do not dereference Cloudflare service bindings on a tokamak native path. tokamak does not currently provide bindings such as D1, KV, R2, Durable Objects, Queues, service bindings, Vectorize, Hyperdrive, Workers AI, Browser Rendering, Images, dispatch namespaces, mTLS, Pipelines, rate limiting, Secrets Store, Email Routing, or Analytics Engine.
- Allow a portable project to declare Cloudflare-only bindings for its web deployment only when native execution does not depend on them.
- Never put a secret in a Wrangler `var` or any other packaged application file. Values and server code shipped in a native app are on the user's device and must be considered inspectable.

## Keep the native trust boundary in view

- Treat the packaged Worker as a local application backend, not as a trusted remote server.
- Put authorization decisions, shared authoritative state, private credentials, and other server-trust responsibilities behind a real remote service when the application needs them.
- Treat browser storage such as IndexedDB or local storage as frontend device storage, separate from the request-scoped Worker runtime. Account for normal browser storage eviction and application removal.

## Design for the actual app surface

- Treat the rendered interface as a web document filling a platform WebView.
- Use CSS pixels and the live viewport rather than physical display pixels or a fixed tokamak canvas size.
- Expect desktop windows to resize. Expect mobile dimensions to vary by device, orientation, system bars, safe areas, display scale, and the on-screen keyboard.
- Include an appropriate mobile viewport declaration, normally `width=device-width, initial-scale=1`.
- Use responsive layout and content-driven breakpoints. Do not equate a target name with one width or hard-code a known phone's dimensions.
- Use safe-area environment values when choosing an edge-to-edge layout with `viewport-fit=cover`; do not hard-code notch or system-bar insets.
- Account for the input capabilities of the chosen targets. Touch, pointer hover, mouse, trackpad, keyboard, and hardware back controls are not universally present.
- Do not rely on browser chrome for navigation, progress, offline status, or recovery. A native app owns the user-visible experience inside its window.

## Treat lifecycle and connectivity as interruptible

- Assume that a mobile operating system can pause JavaScript, suspend the app process, close sockets, fail in-flight requests, or later terminate the app while it is in the background.
- On iOS foregrounding, expect the tokamak shell to restore its local gateway and update its internal proxy if necessary. Do not confuse gateway recovery with recovery of frontend JavaScript state or network sessions.
- Treat every WebSocket, EventSource, streaming response, subscription, and in-flight request as interruptible. A connection object that existed before backgrounding may be stale even if it has not yet emitted a useful error.
- Make long-lived frontend connections self-repairing. Reconnect after close or error and re-evaluate them when the document becomes visible, a page is restored, or the network comes online.
- Treat a reconnected transport as a new session. Reauthenticate where required, recreate subscriptions, and reconcile state or missed events from an application-level cursor or fresh snapshot.
- Retry reads and other safe operations according to their semantics. Do not automatically replay a non-idempotent mutation unless the protocol supplies an idempotency mechanism.
- Recreate page-scoped native plugin subscriptions after navigation or reload.
- Do not depend on timers continuing to run while an app is backgrounded.

## Use native capabilities through tokamak plugins

- Import supported `@tokamak/*` frontend plugins for native capabilities instead of modeling those capabilities as Worker bindings.
- Call plugins from browser-side code, where the native bridge exists. Do not expect the bridge in the packaged Worker handler.
- Preserve a plugin's web implementation or feature-detect availability when the same code also targets ordinary browsers.
- Handle permission denial, unavailable hardware, cancellation, navigation, and page lifecycle as normal outcomes of a native capability request.
- Inspect the installed plugin package before inventing a method, event, permission, or platform fallback.

## Preserve user intent

- Follow the user's requested targets, design, and validation scope. Do not prescribe a platform matrix, a web deployment, or another platform unless the user asks for it.
- State an actual tokamak limitation when it affects the request, then offer choices that fit the application's requirements rather than silently replacing the architecture.
