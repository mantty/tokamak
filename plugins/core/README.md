# @tokamakdev/plugin

Native plugin APIs, endpoint codecs, event contracts and test fixtures.

| Entry | Contents |
| --- | --- |
| `@tokamakdev/plugin` | `Plugin`, `TokamakContext`, JSON codec and value types |
| `@tokamakdev/plugin/streams` | Bounded push streams and typed JSON stream decoding |
| `@tokamakdev/plugin/events` | `defineEvent` |
| `@tokamakdev/plugin/testing` | Invocation fixture and event dispatch |

Plugin packages declare this package as a peer dependency. Native APIs require
the receiving Tokamak worker invocation's context:

```ts
import { Plugin, type TokamakContext } from "@tokamakdev/plugin";

class Alarms extends Plugin {
  constructor(ctx: TokamakContext) { super(ctx, "alarms"); }
  set(at: number): Promise<void> { return this.call("set", { at }); }
  rings(): ReadableStream<Uint8Array> { return this.stream("rings"); }
}
export function alarms(ctx: TokamakContext) { return new Alarms(ctx); }
```

`call` uses the standard JSON endpoint codec. `callBytes` sends and receives
opaque bytes. `stream` exposes JSON values as a cancellable byte stream;
`decodeValues<T>` consumes it. Errors terminate a stream. A slow consumer
exceeding its bounded buffer receives `QuotaExceededError`.

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

The plugin defines each event in its own `/events` entry, which apps import:

```ts
// events/index.ts, exported as "./events" in package.json
import { defineEvent } from "@tokamakdev/plugin/events";

/** Receives each alarm as it rings. A returned number snoozes it for that many minutes. */
export const onRing = defineEvent<{ id: string }, number>("alarms.ring");
```

`defineEvent<T, Reply>(name)` returns the function that registers listeners,
typed with the event's data `T` and the `Reply` its listeners may return. The
name is the plugin's ID, a `.`, and the event's name. The event's reply is the
first value, in registration order, that a listener returns other than
`undefined`.

The native side emits the event through the host the plugin is created with,
which prefixes the plugin's ID, so listeners of `alarms.ring` receive
`host.emit("ring", …)`. Events are JSON:

```swift
host.emit("ring", event: ["id": alarm.id], timeout: 10) { result in
  // `result` is the reply, parsed from JSON, or nil when no listener replies.
}
```

```kotlin
// Blocks, so call it off the main thread.
val reply = host.emit("ring", JSONObject().put("id", alarm.id).toString(), 10_000)
// `reply` is the reply's JSON, "null" when no listener replies.
```

`timeout` includes starting the runtime when the app is not running. `emit`
fails when the Worker cannot run, or when a listener has not returned by the
timeout; it is not retried. The runtime starts no Worker for an event without
listeners, so emit each event the plugin defines whether or not the app
listens. Plugin IDs contain no `.`.

Test listeners with `dispatch` from `@tokamakdev/plugin/testing`, which runs
them as the runtime does and returns what they threw:

```ts
import { dispatch } from "@tokamakdev/plugin/testing";

onRing(() => 5);
const { reply, reported } = await dispatch("alarms.ring", { id: "a1" }, env);
```
