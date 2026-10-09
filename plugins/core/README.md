# @tokamakdev/plugin

The shared base of tokamak plugins: the page's native transport, Worker events,
and test fakes.

| Entry | Contents |
| --- | --- |
| `@tokamakdev/plugin` | `FrontendPlugin`, the base class of a plugin's page API |
| `@tokamakdev/plugin/events` | `defineEvent`, which defines a plugin's Worker events |
| `@tokamakdev/plugin/testing` | A fake native transport, and `dispatch`, which runs event listeners as the runtime does |

A plugin lists `@tokamakdev/plugin` as a peer dependency, so an app has one
copy of it, shared with `@tokamakdev/tok`, which depends on it.

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
