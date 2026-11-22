# @tokamakdev/plugin

Native plugin APIs, endpoint codecs, event contracts and test fixtures.

| Entry | Contents |
| --- | --- |
| `@tokamakdev/plugin` | `platformApi`, `NativeEndpoint`, `TokamakContext`, `platform`, JSON codec and value types |
| `@tokamakdev/plugin/streams` | Bounded push streams and typed JSON stream decoding |
| `@tokamakdev/plugin/events` | `setEventCodec` |
| `@tokamakdev/plugin/testing` | Invocation fixture |

Plugin packages declare this package as a peer dependency. Native APIs require
the receiving Tokamak worker invocation's context. `tok plugin generate` writes
each platform family's API from the plugin's native code, and the plugin's own
API calls the one for the platform the app runs on:

```ts
import { platformApi, type JsonStream, type TokamakContext } from "@tokamakdev/plugin";
import { android } from "./android.generated.js";
import { apple } from "./apple.generated.js";

export function alarms(ctx: TokamakContext) {
  const native = platformApi(ctx, { apple, android });
  return {
    set: (at: number): Promise<void> => native.set(at),
    rings: (): JsonStream<Ring> => native.rings(),
  };
}
```

The generated APIs call native methods through a `NativeEndpoint`. `call` uses
the standard JSON endpoint codec. `callBytes` sends and receives opaque bytes.
`stream` exposes JSON values as a cancellable byte stream; `decodeValues<T>`
consumes it. Errors terminate a stream. A slow consumer exceeding its bounded
buffer receives `QuotaExceededError`.

Declare the factory in `tokamak-plugin.json` alongside native build metadata:

```json
{
  "schemaVersion": 2,
  "id": "alarms",
  "api": { "module": "src/index.ts", "export": "alarms" }
}
```

Tooling generates the `example-alarms/client` default export for pages and the
`env.TOKAMAK_ALARMS` binding for the main worker. Public API methods use the
supported service-value and byte-stream types. Callback parameters, native
objects and arbitrary classes are rejected at build time.

An optional `web` module/export reference supplies an object implementing
frontend methods. Those methods return promises; omitted methods reject with
`NotSupportedError`. Browser code belongs in that module, separate from the
native factory. A native factory never tests browser globals.

In `src/tokamak.ts`, call `alarms(ctx).set(at)` using the context passed to an
event handler, or `alarms(this.ctx)` in a `TokamakEntrypoint` method. Dependencies
and build tooling belong to the same app project.

Tests construct an invocation without installing global transports:

```ts
const native = connectNative();
const result = alarms(native.ctx).set(123);
native.respond({ value: null });
await result;
```

## Showing UI

Calls come at any time, including while the app is off screen. Native code
that would show UI then fails with `NeedsUIError` instead, through the host,
so only calls that would show UI fail:

- **Android:** `requestPermissions` throws `NeedsUIError` when a permission
  would prompt while the app is in the background, and answers at once,
  without UI, when none would. The host remembers the permissions the user has
  refused for good, as Android reports them the same way as permissions never
  asked for; `wouldPrompt` tells them apart. Other UI goes in the
  activity `requireForegroundActivity()` returns, which throws `NeedsUIError`
  in the background; `authenticateOwner` uses it. `isInForeground` reports the
  app's stage.
- **Apple:** `requireUI()` throws `NeedsUIError` on iOS while the app is in
  the background. Call it on the main thread where the plugin decides to show
  UI, and only when the UI would appear, such as before
  `requestWhenInUseAuthorization` while the authorization is not determined.
  A hidden macOS app still shows system prompts, so it never throws there.

## Worker events

A plugin's native code delivers events to the app's Tokamak worker, even when
no page is loaded, such as when the system starts the app in the background.
Each event calls one handler: the function of the event's name in the
property of the worker's class for the plugin, `_` followed by the plugin's
id, such as `_alarms.onRing`. Plugin ids are unique in an app, so two plugins'
events never reach the same handler.

A native plugin lists each event in its registration file, by its name, with
the type of its data and of any reply. `tok plugin generate` writes the
handlers' declarations into the family's platform API, which type the property
an app defines (see [GUIDELINES.md](../GUIDELINES.md)).

Native code emits the event's data through the host the plugin is created with.
The generated dispatch encodes it, and decodes the reply:

```swift
host.emit("onRing", event: alarm, reply: Int.self, timeout: 10) { result in
  // `result` is the reply, or nil without one.
}
```

```kotlin
// Blocks, so call it off the main thread.
val reply = host.emit("onRing", alarm, Int::class, 10_000)
// `reply` is the reply, or null without one.
```

The reply is what the handler returns, when that is not `undefined`.

An event's data and reply travel as JSON. For an event that carries other
bytes, which native code emits with `host.emitBytes`, the plugin's API module
calls `setEventCodec("<plugin id>.<event>", { decode, encode })`, and declares
the handler itself, as the generated code declares its handlers.

`timeout` includes starting the runtime when the app is not running. `emit`
fails when the Worker cannot run, or when the handler has not returned by the
timeout; it is not retried. A packaged app starts no Worker for an event whose
handler the app's class does not define, so emit each event the plugin lists
whether or not the app handles it.

## WebView controls

Inside the Tokamak worker, use `this.ctx.webview.navigate("/recipes")`,
`await this.ctx.webview.getCurrentPage()`, or
`await this.ctx.webview.postMessage({ recipeId: "42" })`. A page receives messages
through its standard `window.addEventListener("message", handler)`.

Navigation accepts URLs within the app's origin and resolves when the native
view accepts the navigation. Messaging resolves after dispatch into the current
page. These operations reject with `InvalidStateError` when no page is loaded;
they neither create a view nor queue messages for a later page.
