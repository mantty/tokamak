# @tokamakdev/plugin

Native plugin APIs, endpoint codecs, event contracts and test fixtures.

| Entry | Contents |
| --- | --- |
| `@tokamakdev/plugin` | `platformApi`, `NativeEndpoint`, `TokamakContext`, `platform`, JSON codec and value types |
| `@tokamakdev/plugin/streams` | Bounded push streams and typed JSON stream decoding |
| `@tokamakdev/plugin/events` | `defineEvent` |
| `@tokamakdev/plugin/testing` | Invocation fixture, event dispatch and `setPlatform` |

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

A plugin's native code delivers events to the app's Tokamak worker, whose listeners
register in `src/tokamak.ts` and run even when no page is loaded, such as when
the system starts the app in the background.

A native plugin registers each event in its registration file, with the type of
its data and of any reply, and `tok plugin generate` writes the function that
registers its listeners into the family's platform API. The plugin's `/events`
entry, which apps import, curates the families' functions into its own (see
[GUIDELINES.md](../GUIDELINES.md)).

Native code emits the event's data through the host the plugin is created with.
The generated dispatch encodes it, and decodes the reply:

```swift
host.emit("ring", event: alarm, reply: Int.self, timeout: 10) { result in
  // `result` is the reply, or nil when no listener replies.
}
```

```kotlin
// Blocks, so call it off the main thread.
val reply = host.emit("ring", alarm, Int::class, 10_000)
// `reply` is the reply, or null when no listener replies.
```

Listeners receive each family's event under the plugin's ID, the family and the
event's name, joined by `.`, such as `alarms.apple.ring`. The reply is the first
value, in registration order, that a listener returns other than `undefined`.

`defineEvent(name, { codec, platforms })` returns the function that registers
listeners of the event `name`, typed with the data and reply that
`Tokamak.Events` and `Tokamak.EventReplies` declare for it. Off `platforms`,
and without a native host, that function registers nothing. The generated API
uses it with its family's platforms. A plugin uses it directly, with a codec,
for an event that carries other bytes than JSON, which native code emits with
`host.emitBytes`.

`timeout` includes starting the runtime when the app is not running. `emit`
fails when the Worker cannot run, or when a listener has not returned by the
timeout; it is not retried. The runtime starts no Worker for an event without
listeners, so emit each event the plugin registers whether or not the app
listens. Plugin IDs contain no `.`.

Test listeners with `dispatch` from `@tokamakdev/plugin/testing`, which runs
them as the runtime does and returns what they threw. `setPlatform` names the
platform under test, on which listeners then register:

```ts
import { dispatch, setPlatform } from "@tokamakdev/plugin/testing";

setPlatform("ios");
onRing(() => 5);
const { reply, reported } = await dispatch("alarms.apple.ring", { id: "a1" }, env);
```

## WebView controls

Inside the Tokamak worker, use `this.ctx.webview.navigate("/recipes")`,
`await this.ctx.webview.getCurrentPage()`, or
`await this.ctx.webview.postMessage({ recipeId: "42" })`. A page receives messages
through its standard `window.addEventListener("message", handler)`.

Navigation accepts URLs within the app's origin and resolves when the native
view accepts the navigation. Messaging resolves after dispatch into the current
page. These operations reject with `InvalidStateError` when no page is loaded;
they neither create a view nor queue messages for a later page.
