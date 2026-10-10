# @tokamakdev/plugin

The shared base of tokamak plugins: the native transport, Worker events, and
test fakes.

| Entry | Contents |
| --- | --- |
| `@tokamakdev/plugin` | `Plugin`, the base class of a plugin's API, which the page and the Worker call alike |
| `@tokamakdev/plugin/events` | `defineEvent`, which defines a plugin's Worker events |
| `@tokamakdev/plugin/testing` | A fake native transport, and `dispatch`, which runs event listeners as the runtime does |

A plugin lists `@tokamakdev/plugin` as a peer dependency, so an app has one
copy of it, shared with `@tokamakdev/tok`, which depends on it.

## Calls

A plugin's API extends `Plugin`, whose `call` and `listen` reach its native
code from the page through the shell's bridge, and from the Worker through the
runtime. `hasNativeTransport` is false outside a tokamak app, such as on the
web and Cloudflare, where the plugin uses its web implementation or rejects
with `NotSupportedError`:

```ts
import { Plugin } from "@tokamakdev/plugin";

class Alarms extends Plugin {
  constructor() {
    super("alarms");
  }

  set(at: number): Promise<void> {
    return this.call("set", { at });
  }

  onRing(next: (id: string) => void, error: (error: DOMException) => void): () => void {
    return this.listen("onRing", next, error);
  }
}

export const alarms = new Alarms();
```

The native side runs each call and subscription on the main thread, for the
page and the Worker alike. A `subscribe` that throws, as for a method the
plugin does not have, ends the subscription; a failure it replies with does
not.

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

A plugin's native code delivers events to the app's Worker, whose listeners
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
