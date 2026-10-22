# Plugin architecture and guidelines

How tokamak plugins are built, and the work to get there. First-party plugins
and plugins an app writes for itself follow the same rules. A plugin that needs
more than these rules allow shows a gap in the runtime or the native shells. The
gap is fixed there, not in the plugin.

## Principles

- tokamak is a tool for developers. A plugin gives a developer the platform's
  capabilities as typed, well-structured JavaScript. It does not decide when or
  how they are used.
- A plugin does not validate what the developer passes, does not persist or
  replay anything on the developer's behalf, does not retry, and does not hide
  platform behaviour. Misuse fails visibly on its own.
- Ergonomic means typed, structured and not verbose. It does not mean guarded.
- Platform behaviour is exposed as it is, and documented as it is.
- The runtime owns mechanics: routing, sessions, cancellation, deadlines,
  decoding, object tables, threading, starting the runtime for a delivery. It
  owns no policy: when to ask for permission, what to persist, what to drop.
- Anything mechanical that can be generated is generated, not written.
- The existing plugins are not models. They show which primitives are needed.
  Their APIs and flows are not carried forward.

## Terms

- **Page**: the app's web UI, running in the WebView.
- **Worker**: the app's server code, running in tokamak's embedded runtime. Each
  request or event runs in a fresh Worker invocation. The page and the Worker
  both call plugins.
- **Native shell**: the Swift or Kotlin application that hosts the WebView and
  the runtime on one platform.
- **Message protocol**: the format and rules of the messages between the page or
  Worker and native plugin code: calls, replies, subscriptions, cancellation,
  events, sessions.
- **Adapter**: the part of a native shell that connects the message protocol to
  the platform: it receives a plugin id, a method name and JSON, and replies
  with JSON or an error.
- **Call**: a plugin method that returns one value, as a promise.
- **Subscription**: a plugin method that delivers a series of values to a
  callback until the caller ends it.
- **Event**: a message native code sends to the Worker without being asked,
  received through listeners registered with `defineEvent`. Sending an event
  starts the runtime when it is not running.
- **Platform module**: the TypeScript module at `<package>/apple` or
  `<package>/android`. Each of its functions maps to exactly one platform API
  call.
- **Curated layer**: the optional TypeScript at `<package>` offering one
  cross-platform API, built by calling the platform modules.
- **Handle**: a string returned in place of a native object, which later calls
  pass back to name that object. The runtime keeps the table from handle to
  object.
- **Platform event**: an event whose name and payload mirror one platform
  callback, such as `location.apple.didEnterRegion`.
- **Launch-time delivery**: the OS starting the app's process to deliver
  something (a region entry, a push, a geofence broadcast) before any page or
  Worker code has run.
- **SDK definitions**: the machine-readable description of a platform's public
  API that its SDK ships: Apple's headers and `.swiftinterface` files per
  framework, Android's `android.jar` and `api-versions.xml`.
- **Generator**: the tool that reads SDK definitions for a chosen set of APIs
  and emits the platform module, the native routing code and the API
  documentation from that one source.
- **`onStart`**: the runtime event from `@tokamakdev/tok/events` that fires at
  every runtime start, including a background launch, before any other event.

## Decisions

1. **The message protocol is implemented once, in Rust, inside the runtime.**
   Today it is implemented in Swift, again in Kotlin, and the Worker's calls
   take a third path. Rust owns routing, sessions, cancellation, deadlines and
   the page and Worker entry points. Each native shell keeps only an adapter.
   Windows and Linux get plugins through the same route, with bodies in C++,
   C# or Rust, once the adapters exist.
2. **Layer one is generated from SDK definitions.** The platform module, the
   native routing code and the platform API documentation come from the
   generator, for the APIs a plugin declares. Hand-written native code is
   limited to what the generator cannot express, listed in Open decisions.
3. **Platform code is Swift and Kotlin.** Rust could reach Apple's
   Objective-C surface through `objc2`, but Apple now ships frameworks with
   no Objective-C headers at all (WidgetKit, ActivityKit, AppIntents,
   GroupActivities, ProximityReader, Translation, FoundationModels in the
   iOS 27 SDK), and that set grows each year. Swift is the language every
   Apple capability is guaranteed to reach, and it carries the compile-time
   OS availability checks (`@available`, and Android lint `NewApi` for
   Kotlin). Rust is the runtime: the message protocol, channels, native
   object tables, streams, and platform-independent work.
4. **The curated layer is optional per plugin** and contains only what more than
   one platform provides.
5. **Subscriptions and events stay two mechanisms for now.** Whether they
   should be one is open decision 1.

## Layers

| Layer    | Import                                 | Content                                                            |
| -------- | -------------------------------------- | ------------------------------------------------------------------ |
| Platform | `<package>/apple`, `<package>/android` | The platform's API, one-to-one, as calls, subscriptions and events |
| Curated  | `<package>`, `<package>/events`        | One cross-platform capability, built from the platform modules     |
| Build    | `tokamak-plugin.json`, `build/`        | What the OS reads from the app package                             |

The platform layer comes first, one platform at a time. The curated layer calls
only the platform modules, browser APIs and the core package. Native code
contains no cross-platform decisions. TypeScript contains no native call
outside the platform modules.

### Platform layer

One module per native API family: `apple` for iOS and macOS, `android` for
Android (open decision 2). The location plugin's Apple module looks like this:

```ts
import {
  createLocationManager,
  requestWhenInUseAuthorization,
  didUpdateLocations,
  releaseLocationManager,
} from "@tokamakdev/plugin-location/apple";

const manager = await createLocationManager(); // Swift: CLLocationManager()
await requestWhenInUseAuthorization({ manager }); // Swift: manager.requestWhenInUseAuthorization()
const stop = didUpdateLocations({ manager }, (locations) => {
  // each delegate call's [CLLocation], as JSON, with Apple's field names
}, (error) => {});
stop();
await releaseLocationManager({ manager });
```

The generator produces this module and the native routing behind it. These are
the rules it applies, and the rules for any hand-written part:

- **Names, parameters, units and enums are the platform's.** A method has the
  platform call's name. Its single argument object has the platform's
  parameter labels as keys. Durations are in the platform's unit. Enum values
  are the platform's case names as strings. Nothing is renamed, converted,
  defaulted or combined.
- **Native objects are handles.** For a platform type with identity and
  lifetime, `create<Type>(init)` makes one, stores it in the runtime's table,
  and returns the handle. Instance methods take the handle as the first field
  of their argument object. `release<Type>(handle)` removes it.
- **Callbacks are subscriptions.** On Apple, each delegate method becomes one
  subscription, named after it, taking the handle, delivering that method's
  arguments each time it fires. On Android, where the callback is a parameter
  of the call itself, that call is the subscription.
- **Permission APIs are platform APIs.** `requestWhenInUseAuthorization` and
  Android's `requestPermissions` are methods like any other, with the
  platform's own result delivery.
- **Platform errors pass through** with the platform's error type, message and
  code (open decision 3).
- **Launch-time delivery is the one thing native code does unasked.** Where the
  OS delivers to an object that must exist at launch, such as a
  `CLLocationManager` created in the app delegate's launch method, the plugin
  creates one at process start under the handle `"default"` and sends its
  callbacks to the Worker as platform events. Native code does nothing else on
  its own.
- **Unavailable means `NotSupportedError`.** A method the OS version or hardware
  lacks rejects with `NotSupportedError`. The generator records availability
  from the SDK definitions; Android lint `NewApi` and Swift `@available` check
  it at build time.
- **No decisions.** The platform layer stores nothing, retries nothing, checks
  nothing before a call, and decides nothing after a delivery.

### Curated layer

Optional. One API that behaves the same on every platform, built by calling the
platform modules:

```ts
import { location } from "@tokamakdev/plugin-location";

await location.setup({ significantChanges: true });
const position = await location.getCurrentPosition({ timeout: 5000 }); // milliseconds
```

- **Built only from the platform modules, browser APIs and the core package.**
  It picks the platform module with `platform()` from the core package.
- **Cross-platform capabilities only.** Something one platform has stays in that
  platform's module. A curated method a platform cannot provide rejects with
  `NotSupportedError`, and its comment names the platforms that reject.
- **Shapes follow the web where the web has the capability.** A position has
  the fields of `GeolocationPosition`; durations and instants are milliseconds;
  distances are metres. Where the web has no such API, the shape is chosen
  once, typed, and named plainly.
- **One typed options object.** The TypeScript types are the contract. There
  are no runtime checks of what the developer passes; a wrong value reaches the
  platform and fails as the platform fails.
- **`setup(config)` applies runtime configuration now and keeps nothing.** An
  app that wants the configuration in force after the OS restarts it calls
  `setup` from `onStart`.
- **The page and the Worker get the same API**: the same methods, values and
  errors.
- **Nothing beyond the call.** No persistence, no retries, no de-duplication of
  what the platform delivers, no permission checks before a call. The
  developer's state lives in the app's own bindings.
- **Platform differences go in the doc comment**, from which the README and
  the API report are generated.

### Events

- `<package>/events` exports the curated events, named `<plugin>.<event>`.
  `<package>/<platform>/events` exports the platform events, named
  `<plugin>.<platform>.<callback>`, one per callback sent at launch-time
  delivery.
- Native code sends platform events only. A curated event is a listener on the
  platform events behind it that translates each one and calls the app's
  listener.
- Delivery is the platform's. Whatever the platform delivers, however often,
  reaches the listener. The event's comment states the platform's documented
  delivery behaviour.
- A listener's return value is the event's reply, taken from the first listener
  that returns something other than `undefined`.
- Each event's comment says which platform mechanisms start the app for it.

## Primitives

What the runtime and the native shells give every plugin, and where each
stands.

| Primitive                                                                                        | Status                                                          |
| ------------------------------------------------------------------------------------------------ | --------------------------------------------------------------- |
| The message protocol: calls, subscriptions, cancellation, sessions, deadlines, in Rust           | Exists in Swift and Kotlin; to move to Rust (decision 1)        |
| One adapter per native shell                                                                     | To write (decision 1)                                           |
| Events from native code to the Worker, starting the runtime when needed                          | Exists: the host's `emit`, `defineEvent`                        |
| Runtime lifecycle events `onStart`, `onResume`, `onSuspend`                                      | Exists: `@tokamakdev/tok/events`                                |
| Build-time settings with typed kinds, and a build script that writes platform declarations       | Exists: `tokamak-plugin.json`, `build/`                         |
| The generator                                                                                    | To write (decision 2); feasibility is work item 1               |
| Handle tables in the runtime                                                                     | To write                                                        |
| JSON decoding of arguments into platform types                                                   | Generated per method                                            |
| `platform()`: which platform the code runs on                                                    | To write                                                        |
| App state: whether the app is in the foreground                                                  | Exists on Android (`isInForeground`); internal only on Apple    |
| App delegate and Activity forwarding: launch options, push token, remote notifications, intents  | Partly: push hooks on Apple, `onIntent` on Android              |
| Presenting native UI: a view controller or activity a plugin shows                               | Missing (open decision 6)                                       |
| Fetching the app's own files or Worker responses from native code                                | Exists: the host's `fetch`                                      |
| A translation path for strings the OS shows                                                      | Exists as a pattern: settings and `setup` values                |
| A fake native transport for TypeScript tests                                                     | Exists: `@tokamakdev/plugin/testing`                            |
| Native test targets run in CI                                                                    | Missing (open decision 8)                                       |

## Package layout

```text
<package>/
  package.json           exports ".", "./events", "./apple", "./apple/events",
                         "./android", "./android/events"
  tokamak-plugin.json    id, settings, build, platforms, the APIs to generate
  README.md
  src/
    index.ts             the curated API, when the plugin has one
    types.ts             the curated types
  events/index.ts        the curated events
  apple/
    index.ts             generated: the platform module
    events.ts            generated: the platform events
    <Name>Routing.swift  generated: method routing and decoding
    <Name>Launch.swift   hand-written: the "default" launch instance, if the API needs one
    <Name>Tests.swift    XCTest for the hand-written part
  android/
    index.ts
    events.ts
    <Name>Routing.kt
    <Name>Launch.kt
    <Name>Test.kt
  web/index.ts           the browser implementation the curated layer uses
  build/index.mjs        the build script, where the manifest declares settings
  tests/*.test.ts        the TypeScript tests
```

Nothing else. A plugin that needs another directory or export changes this
file first.

## Build-time configuration

- A value the OS reads from the app package is a build setting: permissions,
  usage descriptions, entitlements, background modes, receivers and services,
  bundled resources such as sounds. A value a platform API sets while the app
  runs is `setup` configuration.
- Settings are declared in `tokamak-plugin.json` with a typed kind (boolean,
  integer, number, string, path, enum, list, map), a description, and a default
  where one is right.
- The build script writes the platform declarations from the settings. A
  setting the build cannot act on fails the build with a message naming the
  setting.
- A setting one platform reads says so in its description.
- Strings the OS shows come from the app as settings or `setup` values, with a
  documented way to translate them. The plugin bakes in no English.

## Native code

- The generated routing receives a method name and JSON from the adapter,
  decodes the arguments, calls the platform, and encodes the result. A value
  that does not match the declared type is an error with the decoder's
  message.
- Hand-written native code is the launch-time instance and whatever the
  generator cannot express. It uses the primitives in the table and nothing
  else of the shell.
- Native state is the handle table and the `"default"` launch instances.
  Nothing else survives between calls in native memory, and nothing is written
  to native storage.
- Threading: calls arrive on the main thread; a reply may be sent from any
  thread. `emit` differs between Apple and Android today (open decision 4).
- No user-visible strings. Error messages are for developers.

## Calls, subscriptions and connections

- A call returns a promise. A subscription takes `next` and `error` callbacks
  and returns the function that ends it. A plugin adds no transport of its
  own.
- A page's subscriptions end when the page is hidden or navigates away.
- A Worker subscription keeps its invocation alive until it is ended.
- Long-running native work, such as tracking, monitoring, or a connection the
  platform holds open, runs in the platform. Its results reach the Worker as
  events, each in its own invocation.
- Only the OS starts the app: for a region, a significant location change, a
  push, a boot, or a foreground service. The platform module exposes each as a
  platform event.

## Errors

- Every rejection crossing the message protocol is a `DOMException` with a
  name and a message.
- The platform layer passes the platform's error through with its identity.
- The curated layer rejects with `NotSupportedError` where this platform has no
  implementation. Where the web has a standard `DOMException` name for the
  same situation, such as `NotAllowedError` for a refused permission, the
  curated layer uses it, with the platform's error as `cause`. Otherwise the
  platform's error passes through unchanged.
- These reject with `NotSupportedError`: a platform module imported on another
  platform, a curated method the current platform cannot provide, and every
  native call on a platform without an adapter.

## State

A plugin keeps no state beyond the process, and the platform layer keeps only
its handle table. The developer's state lives in the app's own bindings.

## Web, desktop and absent platforms

- On the web, and in the page on Windows and Linux, the curated layer uses the
  browser API the capability has and rejects with `NotSupportedError`
  otherwise. In the Worker on Cloudflare, every native call rejects with
  `NotSupportedError`.
- Windows and Linux have no adapter today. Until they do, every native call
  there rejects with `NotSupportedError`.
- The browser implementation lives in `web/` and is tested with fake browser
  globals.
- Feature detection is the app's job where the same code runs in ordinary
  browsers. The core package exposes `platform()` for it and nothing else.

## Tests

| What                 | Tests                                                                                     | Runs                         |
| -------------------- | ----------------------------------------------------------------------------------------- | ---------------------------- |
| Message protocol     | Rust tests, once, for routing, sessions, cancellation, deadlines                          | CI                           |
| Adapter              | One native test per shell that a call, a subscription and an event cross the boundary     | Simulator, emulator in CI    |
| Generator            | Golden-file tests: a fixed SDK subset produces fixed output                                | CI                           |
| Hand-written native  | XCTest and JUnit for the launch-time instance and anything else hand-written              | Simulator, emulator in CI    |
| Curated layer        | Node tests against the fake transport, asserting the exact platform calls and translation | CI                           |
| Web                  | Node tests with fake browser globals                                                       | CI                           |
| Conformance          | Every public method and event, called from the conformance example app                    | Every platform in CI         |

- A bug fix lands with its failing test first.
- A claim about platform behaviour cites the platform documentation, in the
  test or the doc comment that depends on it.
- Before a device session, write down what the code is expected to do and why.
  The session checks that prediction. Behaviour found only by trying things on
  a device is not a basis for code. What only a device can show is recorded as
  unverified until it is verified.

## Lint

- TypeScript: the workspace eslint config and `tsc`, zero warnings.
- Swift: swift-format and SwiftLint. Kotlin: ktlint, and Android lint with
  `NewApi` as an error.
- Generated files are linted like written ones. A generator whose output fails
  lint is fixed in the generator.

## Documentation

- Doc comments are terse present-tense facts, as `AGENTS.md` requires.
- Platform module documentation is generated with the module, from the SDK
  definitions, and records the SDK version it came from.
- README order: summary and install; the curated API by capability; `setup`;
  events; build settings; platform modules; errors.
- The API report generated from the TypeScript declarations is committed. A
  change to it is part of the pull request that causes it.

## Versioning

- `package.json` follows semver on the public exports. The platform modules are
  public.
- `@tokamakdev/plugin` is a peer dependency. A plugin states the lowest version
  it needs.
- The message protocol and the adapter interface are versioned with the
  runtime. A plugin states the lowest runtime version it needs in
  `tokamak-plugin.json`.

## Work

In order. Each item is one pull request unless stated.

1. **Generator feasibility.** Parse the SDK definitions for Core Location on
   both platforms and emit the platform module, the Swift and Kotlin routing,
   and docs for a chosen subset. Record what the generator cannot express: closures, delegates,
   Swift concurrency, Kotlin coroutines, overloads, generic types, platform
   types with no JSON form. The result decides how much of layer one is
   hand-written.
2. **Message protocol in Rust.** Move routing, sessions, cancellation and
   deadlines into the runtime. Write the Apple and Android adapters. Delete the
   Swift and Kotlin implementations. The page and the Worker take the same
   path. Existing plugins keep working through the adapters.
3. **Core primitives.** `platform()`, handle tables, app state on Apple,
   unified `emit` threading, the launch-time instance convention.
4. **Location plugin rebuilt** on the generator, the protocol and the
   primitives: platform modules first, then a curated layer for what is
   cross-platform. This is the proof of the whole approach.
5. **Conformance app and CI matrix.** The app calls every public method and
   event of every plugin. CI runs it on simulator, emulator, browser and the
   three desktops.
6. **The other three plugins rebuilt**, one pull request each.
7. **Desktop adapters**, after the above.

For a new plugin, or a new capability in one, once the above exists:

1. Name the platform APIs involved and cite their documentation: launch-time
   behaviour, limits, delivery behaviour.
2. Declare the APIs in `tokamak-plugin.json` and run the generator for one
   platform. Write the launch-time instance if the API needs one, with its
   tests.
3. Repeat for the other platform.
4. Write the curated layer and its events, if the capability is
   cross-platform, with tests against the fake transport.
5. Write the web implementation.
6. Add build settings and the build script, where the OS reads anything from
   the package.
7. Add every method and event to the conformance app. Run CI on every
   platform.
8. Write the README. Commit the API report.
9. One pull request.

## Open decisions

1. Whether subscriptions and events should remain two mechanisms or become
   one. Needs its own investigation, including the cost of a Worker
   invocation per delivery, which has not been measured.
2. Platform module naming: `apple` and `android` (API families, matching the
   native source directories), or `ios`, `macos` and `android` (the manifest's
   platform keys) with the Apple modules sharing sources.
3. Platform error identity: the protocol carries only a name and message
   today, so the domain and code need a `cause` field end to end.
4. `emit` threading differs between Apple and Android.
5. Handle lifetime: whether a handle a page created ends with that page's
   session or with the process.
6. Presenting native UI from a plugin: which view controller or activity, and
   how the runtime provides it.
7. Whether `setup(config)` may take a platform-only key where no other platform
   has an equivalent, or whether such configuration always goes through the
   platform module.
8. The conformance example app, the API report tooling, the native lint runs
   and the native test targets do not exist yet.
