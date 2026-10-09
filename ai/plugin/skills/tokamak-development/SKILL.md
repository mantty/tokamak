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
- Expect server-side behavior during `tok dev` to come from the framework's host process. Do not use development mode as proof that a Worker API or Cloudflare binding exists in the packaged tokamak runtime. KV, D1 and R2 data in development lives in Wrangler's local state on the development machine, not on the device.
- Expect native frontend plugins to remain available through the development shell.

### `tok build`

- Run the project's build command with the tokamak Vite plugin active.
- Package the Worker that the build generated, and its assets, into each requested native application under `build/<platform>`.
- Run subsequent same-origin requests entirely through the packaged app and its embedded runtime. Do not assume a Node server, Wrangler process, internet connection, or Cloudflare account is present.

## Install the tokamak CLI

- Add the CLI to the project as a development dependency: `npm install --save-dev @tokamakdev/tok` (or the project's package manager equivalent). Run it with `npx tok`, `pnpm exec tok`, or a `package.json` script.
- Expect the package to install the platform packs the machine can build as `@tokamakdev/platform-<target>` optional dependencies: every host gets `android-arm64`; macOS adds `ios-arm64` and the host-architecture `ios-simulator-*` and `macos-*` packs; Windows x64 adds `windows-x64`. Windows apps can only be built on Windows, and Apple apps only on macOS.
- Do not install with `--omit=optional` or `--no-optional`; that omits the `tok` binary and platform packs. If `tok` reports a missing `@tokamakdev/platform-<target>` on a machine that can build it, reinstall with optional dependencies enabled.
- Apps always add `@tokamakdev/tok` for its Vite plugin. The installer script from the tokamak README is another way to get `tok` and every platform pack, under `~/.local`.

## Use the tokamak CLI

Use these current command shapes:

```sh
tok devices
tok dev <device-selector> --project <project> -- <framework-dev-command>
tok build <platform[,platform...]> --project <project>
tok targets
```

- Require Vite with Cloudflare's Vite plugin (`@cloudflare/vite-plugin`) and add `tokamak()` from `@tokamakdev/tok/vite` next to `cloudflare()` in the Vite plugins (in Astro, `vite.plugins` in `astro.config.mjs`). Supported: React Router, TanStack Start, Astro 6+, RedwoodSDK, Vike, Next.js through vinext, React and Vue apps, and plain Workers (including Hono) with a `vite.config.ts`. Not supported: SvelteKit, Nuxt, Analog, Solid (Nitro), Qwik, Angular, and Next.js through OpenNext.
- The plugin does nothing unless `tok` runs the command, so Cloudflare and web builds are unchanged. `tok build` fails when the build does not report a configuration because the plugin is missing.
- `tok dev` learns the development server's address and the Worker name from the plugin, which reads the name with Wrangler from the Wrangler file in the Vite root and `CLOUDFLARE_ENV`; there is no `--server` option.
- `tok dev` follows the development server when it restarts on a new address, as Vite does when its config changes. Configuration changes apply to the app only after restarting `tok dev`, which says when the configuration changes.
- Use `--skip-project-build` to package the previous `tok build`'s output again.
- Choose a Wrangler environment as for Cloudflare: `CLOUDFLARE_ENV=production tok build ios`. Cloudflare's plugin applies it to the generated configuration, including `vars`; named vars do not inherit, and strings and JSON work. There is no `--env` or `--wrangler` option.
- `build/` caches output by default (`--build-dir PATH` overrides). In GitHub Actions, cache it by OS and tested commit across merge and promotion workflows on the same branch; promote with `CLOUDFLARE_ENV=production tok build ios`. Missing/stale inputs rebuild; Apple env, signing, and build-number changes refresh the bundle and re-sign without recompiling the shell.
- Use the current native platform names: `android`, `ios`, `ios-simulator`, `macos`, and `windows`.
- tokamak packages the Worker modules `wrangler deploy` would upload for the generated configuration: ES modules, `Text` modules (`.txt`, `.html`, `.sql`) as strings and `Data` modules (`.bin`) as `ArrayBuffer`s. `CompiledWasm` and `CommonJS` modules fail the build; the runtime has no WebAssembly.
- tokamak runs the `package.json` build script with pnpm, Yarn, or npm, chosen from the project's lockfile. Use `--build` or `TOKAMAK_BUILD` with a shell command when the project needs a different one, such as a monorepo task runner that also builds shared workspace packages.

### Configuration and settings

- The configuration is the options of `tokamak()` in the Vite config, for example `tokamak({ name: "My App", version: "1.0.0", ios: { "team-id": "ABCD1234" } })`. There is no configuration file and no `--config` option. `type Config` from `@tokamakdev/tok` types a configuration declared apart from the call.
- The Vite config is code: express shared bases and per-environment variants, including `process.env` values, as imports and conditions. There is no `include` and no `tokamak.jsonc`.
- `module`, relative to the Vite root, names the module the entry Worker imports in tokamak builds and development, for Worker code that runs only on the device, such as event listeners. It defaults to `src/tokamak.ts`, then `src/tokamak.js`, when one exists; a `module` that does not exist fails the build. Its top-level code runs whenever the Worker is evaluated (and during Astro prerendering), so keep it cheap and safe. Cloudflare and web builds do not include it.
- Top-level keys are `name`, `identifier`, `icon`, and `version`, which are defaults for every platform, and `module`; other top-level keys are rejected.
- Platform objects (`android`, `ios`, `macos`, `windows`) override `name`, `identifier`, or `icon` per platform, for example `{ name: "My App", ios: { name: "My App Pro" } }`. Their other keys are settings for that platform's pack. `ios` also applies to `ios-simulator`. `version` is top-level only.
- Display names preserve their spelling and capitalization. Tokamak derives a lower-case slug for filenames, application IDs, and local hosts. If `name` is absent, the Wrangler Worker name is used as both name and slug, so it must already be a slug (lowercase letters and digits joined by single hyphens, at most 63 characters); set `name` when it is not.
- `identifier` values are used as the Apple bundle identifier and Android application ID.
- `version` is required by `tok build`. Do not use `TOKAMAK_APP_VERSION`.
- `icon` accepts user-created platform assets: Android `res` directories, Apple `.icon` packages for `ios`/`macos`, and Windows `.ico` files. Missing icons preserve the existing behavior.
- Every setting is a command-line option, an environment variable, or a configuration key, with the same name in each: `--version`/`TOKAMAK_VERSION`/`version`, and `--ios-plist`/`TOKAMAK_IOS_PLIST`/`ios.plist`. Options beat environment variables, which beat the configuration; within a source, a platform's own value beats a top-level one. The build command is the exception: only `--build` or `TOKAMAK_BUILD`.
- tokamak validates `name`, `identifier`, `icon`, and `version`. Every other platform setting belongs to the platform pack, which declares it; `tok build <platform> --help` lists a platform's settings. An undeclared setting for the platform being built is an error; settings for other platforms are ignored. Do not invent platform settings.
- Relative paths in options and environment variables are relative to the current directory; relative paths in the configuration are relative to the Vite root, which is the project directory unless the Vite config sets `root`.
- `tok version` prints the CLI version; `--version` sets the app version.

Examples:

```sh
tok build ios --ios-build-number 5
tok build ios --ios-plist native/Info.plist
TOKAMAK_MACOS_PLIST=native/Info.plist tok build macos
```

Apple platform packs accept optional XML or binary application plists through
`ios.plist` and `macos.plist`. The plist must have a dictionary root. Tokamak
layers its generated values first, then icon values, each plugin's plist, and
the application plist last. Plugin plists merge dictionaries by key and arrays
as a union; a different value at the same key path fails the build unless the
application plist sets that key path. The application plist overlays
everything, replacing arrays and other non-dictionary values, so an app that
sets `UIBackgroundModes` must list the modes its plugins need.

`ios.entitlements` and `macos.entitlements` name entitlements plists layered
over the provisioning profile's entitlements at signing. Each declared
entitlement must be permitted by the profile; automatic signing provisions a
profile that permits them. Declare `aps-environment` (iOS) or
`com.apple.developer.aps-environment` (macOS) for push notifications; signing
uses the profile's value. Entitlements that need a profile require
`macos.team-id` on macOS.

The Android platform pack accepts an optional partial `AndroidManifest.xml`
through `android.manifest`, for example to declare `android.permission.CAMERA`
for `getUserMedia`. The Android Gradle Plugin's manifest merger combines it with
the generated manifest, and the application manifest has the higher priority:
elements merge by key, a conflicting attribute fails the build unless the
application manifest marks it with `tools:replace="android:<attribute>"`, and
`tools:node="remove"` removes an element, including a plugin's permission.

Apple build numbers are the `ios.build-number` and `macos.build-number`
settings. They are optional and use the app version by default.

For physical iOS signing, use either `ios.team-id` for automatic selection or
both `ios.signing-identity` and `ios.provisioning-profile` for manual signing.
Do not provide both modes. Use `tok certs` to inspect installed identities and
profiles. `tok build ios` does not need a device ID; simulator builds do not
require provisioning.

macOS builds are ad-hoc signed unless `macos.team-id` is set. With a team,
Tokamak signs with a macOS development profile for that team that includes
this Mac, provisioning through Xcode when needed. Team signing enables the data
protection keychain, which the secure storage plugin requires on macOS; such
builds run only on Macs registered to the team. Distribution signing (Developer
ID, hardened runtime, notarisation) is not covered.

## Respect the packaged Worker contract

- Export a default Worker object with a `fetch(request, env, ctx)` handler, directly or through a compatible framework adapter.
- React to native events with listeners registered at the top level of `src/tokamak.ts` or the modules it imports, never in request handlers. `@tokamakdev/tok/events` exports `onStart` (once per runtime start, including background launches; `{ foreground }`), `onResume` (each move into the foreground, not at launch) and `onSuspend` (each move out of it). Each plugin exports its own from its `/events` entry, such as `onPush` and `onNotificationOpened` from `@tokamakdev/plugin-notifications/events`. A listener is `(event, env, ctx)` like Cloudflare's `scheduled` handler: `env` is `Cloudflare.Env` from `wrangler types`, and `ctx.waitUntil` keeps the event running. The reply is the first value a listener returns other than `undefined`; `onPush` shows the notification returned.
- An event completes when its listeners and `waitUntil` promises settle, or at its deadline: 10 seconds, or 25 seconds for `onSuspend` on iOS. Events wait for `onStart`, lifecycle events run one at a time in order, a throwing listener is reported and the others still run, and nothing is retried. Each event runs in a fresh Worker invocation, so keep state in KV, D1 or R2. macOS fires resume and suspend only for hiding and miniaturising, Windows only for minimising.
- Under `tok dev`, events reach the development server's Worker through Cloudflare's dev registry, and editing `src/tokamak.ts` replaces its listeners. The registry holds one Worker per name, and a killed development server's registration blocks the next for 90 seconds. Cloudflare and web builds contain no event code.
- Use standard request and response semantics and same-origin routes between the frontend and packaged Worker.
- Expect a fresh JavaScript runtime and module graph for each packaged HTTP request and event. Do not use module globals, singleton objects, or in-memory caches as durable state across requests; use a KV, D1 or R2 binding.
- Treat a WebSocket Worker context as lasting only for that WebSocket connection.
- Treat the packaged `node:fs` view as request-scoped: `/bundle` contains read-only packaged files and `/tmp` is fresh for the request. Do not use it for persistent application data.
- Check tokamak's current support before relying on a specific Cloudflare Worker or Node API. Similar syntax is not evidence that every Cloudflare or Node behavior exists.

## Do not assume Cloudflare bindings exist

- Treat Wrangler `vars` containing text or JSON as the supported Worker environment values.
- Treat Wrangler static assets as files packaged and routed by tokamak before Worker dispatch.
- The assets binding, `env.ASSETS` or the name `assets.binding` gives, serves the packaged assets with Cloudflare's `html_handling` and `not_found_handling`. Where Cloudflare redirects to a path's canonical form, tokamak serves the asset directly; `_headers` and `_redirects` files are not applied.
- Use KV, D1 and R2 bindings for Worker data on native targets. A packaged app backs each `kv_namespaces` and `d1_databases` binding with a SQLite database on the device, and each `r2_buckets` binding with a directory of object files, created on first use. Each behaves as its local Wrangler counterpart does, including errors and per-item limits. D1 databases migrate on launch from the binding's `migrations_dir` and `migrations_pattern`, as `wrangler d1 migrations apply` would. The data is local to the device: it is not synced with the Cloudflare resource the binding names, and removing or repointing the binding deletes it at the next launch.
- Write R2 objects from values of known length, as Cloudflare requires: strings, buffers, `Blob`s, request and response bodies, or a `FixedLengthStream`. Ask `list` for `httpMetadata` and `customMetadata` with `include`. R2 rejects `ssecKey` on the device.
- Do not dereference other Cloudflare bindings on a tokamak native path. tokamak does not currently provide bindings such as Durable Objects, Queues, service bindings, Vectorize, Hyperdrive, Workers AI, Browser Rendering, Images, dispatch namespaces, mTLS, Pipelines, rate limiting, Secrets Store, Email Routing, or Analytics Engine; `tok build` warns about each one the generated configuration declares.
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

- Import supported `@tokamakdev/*` frontend plugins for native capabilities instead of modeling those capabilities as Worker bindings.
- Install plugins as project dependencies, for example `npm install @tokamakdev/plugin-location`. `tok build` and `tok dev` include native code for plugins listed in `dependencies`, `devDependencies`, or `peerDependencies`, found in the nearest `node_modules` of the project or a parent directory.
- Call plugins from browser-side code, where the native bridge exists. Do not expect the bridge in the packaged Worker handler; native code reaches the Worker through events instead.
- Preserve a plugin's web implementation or feature-detect availability when the same code also targets ordinary browsers.
- Handle permission denial, unavailable hardware, cancellation, navigation, and page lifecycle as normal outcomes of a native capability request.
- Inspect the installed plugin package before inventing a method, event, permission, or platform fallback.
- First-party plugins are `@tokamakdev/plugin-location`, `@tokamakdev/plugin-secure-storage` (device-only secrets, optionally bound to Face ID, fingerprint or passcode), `@tokamakdev/plugin-local-authentication` (device owner checks the app performs when it chooses) and `@tokamakdev/plugin-notifications` (local and push notifications on every platform, including the web).
- Push needs per-platform declarations: `aps-environment` in the `ios.entitlements` file, `com.apple.developer.aps-environment` in the `macos.entitlements` file with `macos.team-id`, the Firebase project values as `<meta-data>` in the `android.manifest` file, and a service worker using the plugin's helper on the web. The app's server stores subscriptions and sends through APNs, FCM or Web Push.
- Camera and microphone use the standard `getUserMedia` API, not a plugin. Declare them per platform: `NSCameraUsageDescription`/`NSMicrophoneUsageDescription` in the `ios.plist` or `macos.plist` file, and `android.permission.CAMERA`/`android.permission.RECORD_AUDIO` in the `android.manifest` file.
- Android builds run lint's `NewApi` check over shell and plugin Kotlin. Guard calls to APIs newer than the minimum SDK with a direct `Build.VERSION.SDK_INT` comparison; an unguarded call fails the build.

## Preserve user intent

- Follow the user's requested targets, design, and validation scope. Do not prescribe a platform matrix, a web deployment, or another platform unless the user asks for it.
- State an actual tokamak limitation when it affects the request, then offer choices that fit the application's requirements rather than silently replacing the architecture.
