# Plugins and the Tokamak worker

Status: architectural direction agreed on 2026-10-18. This is an implementation
plan, not a claim that the architecture is implemented. It replaces the
2026-10-14 proposal. The seven outstanding architectural decisions were resolved
on 2026-10-19 and are summarised under Decisions. Implementation and device
validation remain to be done.

## The model

**The Tokamak worker is the home for native app behaviour.** It contains
plugins, lifecycle listeners, background event handlers, and the user's other
Tokamak-specific code. It is the only application-code context with a direct
connection to the native device.

The application has three roles:

- **Page:** the UI. A generated, typed SDK calls the Tokamak worker in native
  apps, or uses a plugin's optional browser implementation in web deployments.
- **Main worker:** the application's normal Cloudflare-compatible backend.
  It accesses the Tokamak worker through a service binding when needed.
- **Tokamak worker:** a generated Worker bundle containing the selected
  plugins and the user's native-specific module. It can call platform APIs,
  handle native events, control the WebView, and communicate with the main
  worker.

The Tokamak worker uses the familiar Worker programming model and QuickJS
engine. The existing invocation and native-event machinery can be reused, but
hosting a graph of Worker bundles and providing service bindings/RPC is a new
runtime subsystem. It includes transferring values and streams between separate
QuickJS runtimes. A persistent context per page or WASM is not a prerequisite.

```text
Native application

Page -- generated SDK / HTTP ------> Tokamak worker <----> Native device APIs
                                      ^       ^
Main worker -- service binding -------+       +--- native events / lifecycle
                                      |
                                      +----> Main worker APIs / bindings
                                      +----> WebView control / page messages

Web deployment

Page -- generated SDK -------------> Optional browser implementation
Page -----------------------------> Main worker on Cloudflare
```

The native route always goes through the Tokamak worker, including when a
browser API also exists in the native WebView. Browser fallback code belongs
to the web deployment path, not inside either worker.

## Why this split

Background handlers are application code. A location update may require a
credential from secure storage before it can be uploaded. The handler must
therefore be able to use other plugins, not merely receive a payload.

Putting that code in the Tokamak worker gives it native capabilities without
depending on a running page or adding native transports to the main worker.
Frontend callers and background handlers use the same native implementation.
Browser support remains possible without making every plugin implement a
browser equivalent.

The goal is a small, understandable model, easy plugin authoring, access to
low-level native capabilities, and good performance. Choosing this architecture
does not by itself prove performance.

## User code and generated wiring

Use `src/tokamak.ts` as a separate worker entrypoint in the same application
project. The application's normal entrypoint builds the main worker;
`src/tokamak.ts` builds the Tokamak worker. It is no longer an extra module
injected into the main worker.

Both entrypoints use the application's existing `package.json`, lockfile,
installed dependencies, dependency resolution, and build tooling, including
applicable aliases and transforms. Developers use the same build and local-dev
commands, with no separate project, dependency installation, or manually
maintained worker build configuration.

User code imports runtime-compatible third-party packages and shared application
modules normally. Each worker bundle includes the dependencies reachable from
its entrypoint. Shared imports reuse source code, not live state; the workers
execute separately and communicate through bindings.

This module, and modules it imports, contain:

- App startup, resume, suspend, and other Tokamak lifecycle handlers.
- Plugin background handlers, such as location updates and incoming pushes.
- Native setup and application-specific device behaviour.
- User-defined operations exposed to the page or main worker.

The generated worker includes the installed plugins selected for the app,
native connectivity, and the routing needed to expose their declared APIs.
Tooling generates the extra bundle, service wiring, frontend clients, and
types. Developers should not write a second collection of transport wrappers
or repeat every plugin method in an application router.

Move the existing event-entrypoint loading and dispatch machinery from the
main worker to the Tokamak worker. Incoming lifecycle and plugin events invoke
the Tokamak worker's `event` entrypoint, starting an invocation when needed,
without depending on a page or a request to the main worker. This reuses the
existing event mechanism with a new target worker.

The `src/tokamak.ts` module and its listener modules must be loaded before an
event is dispatched. Registering a listener when a module is evaluated must
not itself fire a lifecycle event: app startup and JavaScript bundle evaluation
are different occurrences.

A page callback remains page code. A background handler is declared in the
Tokamak worker's module graph; closures are not extracted from page code or
silently moved between contexts.

On web deployment, omit the Tokamak worker and its native-only module graph.
The main worker retains its Cloudflare-compatible runtime contract. A main
worker operation that calls a native service binding is still native-dependent;
standard binding syntax does not make that capability available on Cloudflare.

## Entrypoints and native context

The default export of `src/tokamak.ts` extends `TokamakEntrypoint`, following
Cloudflare's `WorkerEntrypoint` model. It exposes the receiving worker's
bindings as `this.env` and a host-supplied `TokamakContext` as `this.ctx`.

Support three invocation forms:

| Entrypoint | Called by | Contract |
| ---------- | --------- | -------- |
| `fetch(request)` | HTTP callers, including generated frontend clients | Receives a `Request` and returns a `Response`, or a promise of one. |
| Public RPC methods | A worker service binding | Receive application arguments and return results; native context comes from `this.ctx`, not the caller. |
| `event(event)` | Tokamak's native event dispatcher | Receives one typed `TokamakEvent`, covering lifecycle and plugin events, and uses `this.ctx`. |

Reserve `event` for host dispatch. It is not exposed as an ordinary RPC method
or a generated frontend operation. Its promise and optional result use the
existing event completion/response contract. Typed listener registration can
sit behind this entrypoint; developers need not write a large switch statement.
Listener callbacks receive the current invocation's `TokamakContext` as an
argument. Loading a listener module registers its callbacks; dispatch supplies
the context when invoking them.

Every native plugin SDK, low-level platform SDK, and core device API requires
this context. A plugin binds to it explicitly, for example
`location(this.ctx).getCurrentPosition()`. Shared helpers can accept a
`TokamakContext` parameter and use `location(ctx)` in the same way. The code
need not physically live in `tokamak.ts`; it needs the context supplied to a
Tokamak worker invocation.

An ordinary worker's context does not satisfy `TokamakContext`, so typed calls
with that context, or without a context, fail type checking. The context carries
the actual native connectivity; types describe that dependency rather than
granting access themselves. Importing an SDK grants no native connection, and
there is no implicit global fallback. Import-chain restrictions are not the
mechanism for determining whether a native call is valid.

The context belongs to the receiving invocation. It is not an RPC argument or
result, and callers do not acquire it through the frontend SDK or service
binding. Generated native-side handlers bind plugin calls to their own context.
The same rule applies to HTTP, RPC, lifecycle, and plugin-event invocations in
packaged apps and local development.

Follow Cloudflare's RPC visibility convention: public prototype methods are
operations; use JavaScript `#private` methods for internal helpers. `fetch` and
`event` are reserved handlers, not generated RPC methods. Generate app service
types and frontend clients from this class without decorators or a second
handwritten operation list.

## The SDK surfaces

### Package conventions

Keep each plugin in one package, installed through npm, pnpm, or Yarn as today,
with conventional subpath exports:

| Export | Purpose |
| ------ | ------- |
| `@tokamakdev/plugin-<name>` | Cross-platform native SDK, bound with a `TokamakContext`. |
| `/apple`, `/android`, `/windows`, `/linux` within that package | One-to-one platform SDKs, also requiring the context. |
| `/client` | Generated frontend client, including any declared browser implementation. |
| `/events` | Typed plugin listener registration. |
| `@tokamakdev/tok` | `TokamakEntrypoint` and context/event types, including the `ctx.webview` interface. |
| `@tokamakdev/tok/events` | App lifecycle listener registration. |
| `@tokamakdev/plugin` | Shared plugin-authoring helpers. |
| `virtual:tokamak/client` | Generated frontend client for the app's own public RPC methods. |

The existing build integration supplies the virtual module and its editor
declarations. Plugin clients are package exports; app-specific clients are
generated from `src/tokamak.ts` within the same project.

### Tokamak worker: platform SDKs

Provide one-to-one SDKs for individual native platforms and API families.
Preserve platform names, concepts, units, methods, callbacks, and object
identity as far as the JavaScript/native boundary permits. These are the
low-level escape hatch when a higher-level plugin API is insufficient.

Use a shared `/apple` export for Apple's SDK families, with iOS/macOS and
minimum-version availability recorded per symbol. Use the other platform
exports for their corresponding SDKs. A shared package does not imply that
every symbol is available on every OS or version.

Generate bindings from the platform API descriptions where practical. Permit
plugin authors to add native code for APIs or behaviours that metadata cannot
express. A plugin must not be limited to what the curated API happens to expose,
or require new runtime plumbing for each native method.

These SDKs require a `TokamakContext` to access the platform. The main worker
uses the Tokamak worker's service contract, and the frontend uses its generated
client.

### Tokamak worker: cross-platform native SDKs

Offer a higher-level API built on the platform SDKs, such as getting a location
or reading a secure value. This layer is native-only. It contains native
platform selection and useful shared behaviour, never browser fallbacks.

Plugin authors should provide convenience without obscuring native errors or
removing the low-level escape hatch. TypeScript types describe the contract;
avoid adding policy, coercion, or defensive abstraction that the operation
does not need.

Cross-platform does not mean that every operation is supported by every native
OS. Platform support and foreground/UI requirements remain part of the API's
declared contract.

### Frontend: generated, thin, typed SDKs

Generate frontend clients from the Tokamak worker's exposed contracts,
including plugin operations and user-defined operations. The native
implementation stays in the Tokamak worker. The frontend client only carries
arguments, results, errors, cancellation, and supported subscriptions or
streams across the worker boundary.

A plugin may supply browser fallback code for the generated frontend SDK.
For example, the web implementation of getting location can call
`navigator.geolocation`. The generator consumes that implementation; it does
not invent a fallback from the native method's signature.

The routing rule is:

- Native app: call the Tokamak worker.
- Web deployment with a declared fallback: call the browser implementation.
- Web deployment without a fallback: report that the capability is unsupported.

Do not restrict the frontend SDK to methods with browser equivalents. A native
page can call a native-only method through the same generated client. Also,
do not treat failure of a native call as a reason to silently use a browser
fallback: deployment target selects the implementation.

Generate per-method editor documentation listing supported platforms, minimum
OS versions, browser fallback availability, and foreground/UI requirements.
Both a bound native SDK and its frontend client expose a readonly
`capabilities` map: for example, `location.capabilities.getCurrentPosition`
reports whether this implementation is supported on the current target. It
does not report permission grants or promise that a call will succeed.
Reserve that member name in generated APIs. Unsupported operations reject
with `NotSupportedError`; support is not inferred from an import path.

Web bundles must not pull in the native worker SDKs, native implementation, or
device relay credentials.

### Main worker: a service binding

The main worker accesses exposed Tokamak operations through a Worker-style
service binding, using the HTTP and RPC subset specified below. Generate its
types from the same contract as the frontend client; do not advertise the full
Cloudflare RPC surface.

The main worker receives neither a `TokamakContext` nor the native bridge, so
it cannot directly use platform SDKs. Calling the Tokamak worker is the explicit
boundary between portable backend code and native behaviour.

### Service graph and routing

**This subsystem does not exist in the packaged runtime today.** In
`tokamak/src/builtins/cloudflare-workers.mjs`, `ServiceStub.fetch` and `connect`
throw unsupported errors, and `WorkerEntrypoint` only stores `ctx` and `env`.
`Dispatcher` currently owns one Worker bundle. Neither those class exports nor
working Miniflare bindings provide packaged multi-worker execution.

Generate `env.TOKAMAK` in the main worker for the app's public Tokamak methods,
and `env.MAIN` in the Tokamak worker for the main worker's HTTP/RPC interface.
Each plugin's declared service API also gets a generated named entrypoint and
binding, such as `env.TOKAMAK_LOCATION`. This exposes plugin operations without
requiring app-written forwarding methods or passing a native context to callers.
Derive names from plugin IDs and fail the build on binding/name collisions.
The same named services back the frontend clients. Native-only bindings are
absent in Cloudflare deployments. Mark them optional in the main worker's
shared environment types so portable code checks availability before calling.

Implement the embedded host's service registry, per-service bundle/bindings,
default/named entrypoint resolution, and asynchronous call dispatcher. Calls
route in-process without loopback HTTP, creating a receiving invocation with
its own QuickJS runtime, context, and bindings. No QuickJS value or object
reference can be passed directly from one runtime to another: implement
encoding/reconstruction and host-owned stream channels for the contract below.
The existing in-runtime `structuredClone` is not a cross-runtime serializer.
Keep dispatch asynchronous so nested calls do not hold a service lock while
waiting for another invocation.

### Initial service-binding and RPC contract

This is an explicit subset of Cloudflare's API. It applies in both directions
between the main and Tokamak workers, to generated plugin entrypoints, and to
generated frontend clients. Public methods are asynchronous at the caller:
return an ordinary `Promise<T>` and await the callee's result before sending it.
Support public prototype methods on default and named entrypoints; do not
expose remote properties/getters or promise pipelining. `event` remains host-only.

The allowlist below is the contract for the runtime, generator, and dev/frontend
adapters. Ordinary data crosses by value; readable streams transfer ownership.
Remote behaviour does not cross this boundary. An exposed signature outside
the allowlist is a generation error, even if Miniflare can execute it.

| Surface | Initial support |
| ------- | --------------- |
| HTTP binding | `fetch` dispatches to the selected entrypoint, recreating the `Request` and `Response` and streaming their bodies. |
| Scalar RPC values | `undefined`, `null`, booleans, strings, numbers including non-finite values, and `bigint`. |
| Structured RPC values | Arrays/tuples, plain records, `Date`, `RegExp`, `Map`, and `Set`, recursively containing supported values. Preserve cycles and repeated references within each transferred graph, not identity across calls. |
| Binary RPC values | Non-shared `ArrayBuffer`, typed arrays, and `DataView`. Copy their data and preserve view relationships within the graph; there is no caller-supplied transfer list. |
| Errors | `Error` and `DOMException`, as values or thrown/rejected results. Reconstruct `name`, `message`, and supported own data fields such as `cause` and `code`; custom prototypes and the original remote stack are not part of the contract. |
| RPC streams | Byte-oriented `ReadableStream<Uint8Array>` arguments/results, including inside supported containers. Ownership moves to the receiver; the sender cannot read the same stream afterwards. Host channels propagate demand, cancellation, EOF, and errors with bounded buffering. |

The structured-value list is an allowlist, not a promise to support every
browser structured-clone type. Recreate records as data; do not transfer class
behaviour or property descriptors. Reject functions/callback arguments,
`RpcTarget`, passed or returned RPC/service stubs, arbitrary class instances,
symbols, weak collections, `SharedArrayBuffer`, and unlisted host objects.
Reject `WritableStream`/`TransformStream` objects, object-mode readable streams,
and nested promises. `Request`/`Response` are supported by the binding's `fetch`
path, not as arbitrary RPC parameters/results. `ServiceStub.connect` and
WebSocket upgrades through service bindings are outside this initial subset.
WebSockets may still carry the frontend adapter's stream transport.

Errors with unsupported field values fail serialization visibly; they must not
leave a call pending. SDK callback conveniences run in the caller and cannot
send closures over RPC. Logical streams of typed plugin values use shared
codec helpers over the byte-stream boundary; the service signature remains a
byte stream. These limits do not restrict native APIs used within the Tokamak
worker itself.

Define invocation ownership as part of this subsystem. A call is tied to its
caller's invocation. Keep the callee alive until its result settles, its
transferred streams finish/cancel, and its `waitUntil` work settles within the
applicable deadlines. Caller teardown ends the pending reply and cancels transferred
streams. Registered `waitUntil` work can extend the receiving execution context
within its deadline, following Cloudflare's context lifetime model. It does not
selectively cancel other JavaScript promises in that context; unregistered work
does not itself keep the invocation alive. Outbound calls end with their owning
context. OS expiry or runtime shutdown cancels everything in that invocation. Close
channels and release invocation-owned native resources on every exit path.
There are no remote-object/stub leases to retain a callee after completion.

### Generator enforcement and development parity

Check every exposed method's parameters and awaited result recursively against
the allowlist. Accept inferred or declared concrete types, including supported
unions and recursive data shapes. Reject signatures containing unsupported
types or unconstrained `any`, `unknown`, `object`, or generic parameters, with a
diagnostic naming the method and offending parameter/result path. JavaScript
authors can provide checked JSDoc or declarations. Private worker helpers are
not subject to this transport restriction.

Resolve the actual entrypoint base and discover only application prototype
methods, including inherited application methods. Framework handlers are not
app operations. Reject Cloudflare's reserved RPC names (`connect`, `dup`,
`constructor`, `alarm`, and `webSocketMessage`/`webSocketClose`/`webSocketError`)
and the promise-like `then` name. Treat `fetch` and `event` as their dedicated
handlers.

A `void` or `Promise<void>` method returns `undefined` across the boundary.
For example, reject `onChange(callback: () => void)` at `onChange.callback`
and `open(): RpcTarget` at `open.result`; neither generates a client method.

Static types cannot prove actual values, error fields, or stream construction.
The shared boundary implementation must also reject unsupported values with
`DataCloneError` and check byte-stream eligibility/chunks, including for plain
JavaScript and type assertions. This checks transport representability, not
application/domain input validity. Fail an invalid argument before invoking
the receiver; reject an invalid result and release any provisional resources.

Apply the same contract checks to generated dev binding/client adapters.
Miniflare's broader RPC support must not make a function stub or `RpcTarget`
work in `tok dev` when the packaged runtime would reject it. Generate ordinary
promise-returning method types, not unrestricted Cloudflare stub types. Both
the native serializer and the dev/frontend adapters must pass the same value,
error, stream, rejection, and teardown conformance cases.

### Frontend routing

Reserve the same-origin `/_tokamak/` namespace for generated frontend HTTP
adapters, routed by the host before the main worker or app-defined `fetch`.
Generate routes from the same exposed methods as service bindings. Adapters
carry structured arguments, binary values, errors, and supported streams;
they do not force this boundary onto the native bytes protocol. Reject API
types outside the initial RPC contract during generation rather than silently
changing their meaning. Preserve that contract over HTTP, including cycles,
binary values, and errors; JSON stringify/parse alone is insufficient. Keep
binary payloads binary.

Expose streams using Web Streams, with cancellation and backpressure propagated
through the adapters. This is the JavaScript interface, not a restriction on
transport: adapters can use WebSockets for bidirectional communication, with
explicit flow control to preserve the stream contract. The host owns routing
and transport connections; each page session owns its outstanding calls and
subscriptions. Disconnect or navigation cancels that session's work, not
independent native registrations.
Do not silently replay calls after reconnecting.

## Native connectivity and app control

Move all application-facing native connectivity into the Tokamak worker:
device calls, event ingress, native object access, subscriptions, native-facing
WebSocket connections, and development relay state or credentials.

Remove the corresponding injection, connection setup, native globals, and
direct plugin dispatch from the main worker. The main worker retains ordinary
HTTP and WebSocket functionality; it loses the Tokamak-specific device channel.
The page likewise reaches device capabilities through the Tokamak worker.

The native shell and embedded runtime still implement the host side of these
connections. "Only the Tokamak worker" describes which application-code context
gets the capability, not a requirement to move native host implementation into
JavaScript.

Expose core app capabilities as typed objects on the invocation's context:

- **WebView:** `ctx.webview.navigate(url)`, `ctx.webview.getCurrentPage()`, and
  `ctx.webview.postMessage(value)` are asynchronous host calls for the app's
  WebView. `getCurrentPage()` returns the committed main-frame URL, or `null`
  when no page is loaded. Native UI work runs on the platform's required thread.
  In an entrypoint method these are accessed through `this.ctx.webview`.
- **Unavailable pages:** navigation or messaging rejects with
  `InvalidStateError` when its required view/page connection is unavailable.
  Do not create/foreground a view or durably queue messages implicitly. A
  successful `postMessage` means accepted for delivery, not that page JavaScript
  has run. Operations requiring foreground UI report `NeedsUIError` when it is
  unavailable.
- **Main worker:** ordinary calls use the service-binding object
  `this.env.MAIN`, through `fetch` or public RPC methods. A call starts the
  receiving invocation as needed; awaiting it coordinates completion. No
  additional worker-management operations are currently required. Any actual
  host-level controls belong on a typed binding-like capability object.

These are capabilities for application code in `tokamak.ts` and its imported
helpers. Plugins do not need to know about WebViews to expose device APIs.
A handler can complete useful work without a page; app-control capabilities
declare any foreground/UI requirements.

## Native transport

Use a single bytes-based transport between native platform code and the
Tokamak worker for calls, responses, errors, events, and stream chunks. The
transport handles routing and lifecycle metadata; it carries plugin payloads
as opaque bytes without interpreting their contents.

Keep call/stream identifiers, success or failure, and completion flags in
transport metadata. Routing, settling a call, or ending a stream must not
require parsing its payload. Pass payload buffers through intermediate layers
without decoding and re-encoding them or converting them to strings.

The worker SDK and native endpoints encode and decode their payloads.
Application code continues to use typed functions and values. JSON can be an
endpoint codec for structured data, while binary data travels as bytes without
compulsory JSON/base64 conversion. Both use the same transport contract in
packaged apps and local development; there are no separate JSON and binary
plugin transports.

This contract applies only to native platform code communicating with the
Tokamak worker. Frontend-to-worker HTTP and main-worker-to-Tokamak-worker
service bindings retain their own communication mechanisms and encodings.

## Calls, streams, events, and ownership

Keep the useful interaction model from the earlier plan:

- **Call:** one request with a result or error. Remote calls are asynchronous,
  with cancellation propagated where the native API supports it.
- **Stream:** repeated values or bytes in either direction, with explicit
  cancellation and bounded buffering/backpressure appropriate to the producer.
  Cross-worker RPC streams use the byte-stream contract above; SDK helpers can
  encode/decode typed values at the endpoints.
- **Event:** an occurrence delivered by native code or the host to a Tokamak
  worker handler, including a fresh OS event that wakes the app.

Calls from the page and main worker enter the same Tokamak worker service.
Native events enter the service directly and can use all its plugin and core
APIs.

Preserve the existing handler-awaiting, `waitUntil`, deadline, error-reporting,
and response semantics when moving dispatch into the Tokamak worker's `event`
entrypoint and adapting it to the bytes-based transport. The current worker
logic already awaits listeners, drains `waitUntil` work within the deadline,
and delivers the outcome and optional reply to native code. A handler can
complete without returning data.

Retain typed listener replies and the existing selection rule: the first
non-`undefined` return value in registration order supplies the reply. Native
code applies any returned result and completes the OS callback within its
deadline.

For an OS callback requiring an immediate synchronous result, the plugin's
native adapter answers using the platform default or explicit configuration
previously supplied by the app. Expose that configuration through the plugin's
typed SDK, and document the initial/default behaviour. Where useful, deliver
an event afterwards; its reply cannot change the result already returned.
Do not wait for a Worker invocation inside a synchronous OS callback.

Some APIs separately accept a later completion result. For example, Android's
`JobService.onStartJob` returns `true` immediately to keep the job active, then
`jobFinished` reports completion after the work finishes. Route that later
outcome through the existing event completion/response contract; the immediate
return does not replace it. Async follow-up is specific to the native API,
not an additional event stage required of every plugin.

Fresh wake events and ongoing streams are distinct. Receiving microphone
buffers during an active session is not evidence that the OS will wake a
suspended app for arbitrary work. The Tokamak worker uses the execution window
the OS grants; a pending promise, open connection, or stream cannot extend it
indefinitely.

Use invocation ownership by default. Native objects and subscriptions have
explicit close/cancel operations and are released at invocation teardown;
garbage collection is a backstop. Returned streams retain their resources for
their supported stream lifetime and release them on completion or cancellation.
Page disconnect/navigation cancels page-owned work.

Long-lived background registrations are explicit plugin operations, identified
by stable IDs and owned by native code. They can outlive an invocation or page.
A later invocation refers to the same registration by ID, not by retaining a
JavaScript object or a previous invocation's context. Re-registering an ID
updates that registration instead of creating a duplicate.

Operations that intentionally leave a native screen or hardware session open
likewise declare app ownership and return a session ID with an explicit
close/stop operation. Completing the initiating call does not end that session.

After process restart, use the platform's persisted registration where it
exists; otherwise the app's startup/setup handler registers it again. Each
plugin documents which applies. Do not automatically replay arbitrary native
calls or pretend JavaScript handles survive a restart. Reacquire native objects
from stable domain IDs where the platform supports that.

Native object references remain local to their owning runtime. Service
contracts use supported values, streams, or explicitly managed handles;
raw native pointers do not cross to a page or the main worker. Late replies,
cancellation, disconnects, and native failures must settle outstanding work
and release its resources without cancelling independent app registrations.

## Plugin authoring and generation

Easy plugin creation is a requirement for the ecosystem. Authors implement
capabilities; Tokamak supplies the mechanical integration. The authoring
workflow must work outside the Tokamak repository using public packages and
documentation, without knowledge of runtime internals.

Provide a small package scaffold and an example app. The minimum workflow is:

1. Create a normal npm package from the scaffold.
2. Write the capability implementation and its typed public API.
3. Declare supported platforms and any necessary native build settings.
4. Install it as a workspace dependency in the example app, test with `tok dev`
   and a packaged build, then publish through the normal npm workflow.

Authors need only the layers their capability uses. A plugin can support one
platform; cross-platform convenience APIs and browser fallbacks are optional.
A plugin composing existing platform bindings can be written entirely in
TypeScript, without adding native source. Workspace development must not
require publishing after each edit.

A plugin supplies:

1. Its implementation, using existing platform bindings or native code, and
   any required platform metadata.
2. Its optional higher-level cross-platform native API.
3. Its typed callable API and event contracts where events are needed.
4. Optional browser implementations attached to the frontend contract.

Tooling supplies the shared adapter, generated native dispatch, routing,
frontend clients, service bindings, and types. Supply standard endpoint codecs
for structured values and binary data: the single bytes transport must not
require ordinary plugin authors to write serializers or transport wrappers.
Use exported TypeScript signatures as the source for callable/event types;
platform SDK descriptions remain the source for generated one-to-one bindings.
Extend the existing `tokamak-plugin.json` with versioned module/export
references for the native API factory, event contracts, optional browser
implementation, and per-operation availability/interaction metadata. Do not
duplicate method signatures in the manifest or handwritten transport schemas.

The generator binds the declared API factory to the receiving invocation's
context and exposes its public service methods. Each method supplied by the
optional browser module implements the corresponding frontend signature;
omitted methods are unsupported on the web. Validate these signatures during
the build and generate clients, routes, named entrypoints, binding types, and
support documentation together. Enforce the initial RPC allowlist on exposed
service signatures during both development and build. Low-level native objects
stay in the Tokamak worker; exposing an operation that uses them is an ordinary
public app method.

Retain support for native screens, binary data, streams, and native object
lifetimes. A plugin can use platform UI without requiring its consuming code
to run in the page. The native adapter owns thread dispatch and presentation.

Use the package exports above and the existing plugin build system. Do not
require every plugin author to write Rust or a custom protocol.

Only plugins with native events implement the corresponding platform callback
mappings, including any native completion action. Tokamak owns shared event
dispatch, response correlation, deadlines, and resource-cleanup mechanics.
Authors remain responsible for the capability's actual platform behaviour,
permissions, and lifetimes.

## Packaged apps and local development

### Packaged native apps

Build the main worker and Tokamak worker as separate bundles. Include only
the selected native plugins and generated SDKs needed by the application.
Implement the new service subsystem to host both bundles, resolve their
entrypoints/bindings, and transfer supported values and streams between their
invocations. Keep each invocation's existing isolation, request-scoped state,
and native-capability boundary. Packaged tests must prove this independently
of Miniflare's existing multi-worker implementation.

Native startup and background wake paths initialise the Tokamak worker's
entrypoint and deliver events without waiting for page creation or a request
to the main worker. Page callers connect to the already defined service;
they do not create a new plugin worker for each page.

### `tok dev`

Local development is a requirement of the architecture, not a later adapter.
Keep the same logical two-worker graph, entrypoints, SDKs, and event contracts
as packaged apps.

Run the Tokamak worker as a second host-side Worker in the existing
Vite/Cloudflare/Miniflare development environment. Generate its auxiliary-worker
configuration and service bindings through the existing build integration.
The frontend and main worker continue using the same framework dev server;
developers maintain one project and run the same command.

Move the existing development native adapter, relay connectivity, and event
dispatch from the main worker into this Tokamak worker. The device's native
shell forwards lifecycle/plugin events to it; its native calls return through
the existing device/session relay. Only the Tokamak worker receives that
connectivity. Main-worker calls use normal local service bindings, and frontend
clients reach its generated routes through the existing dev-server proxy.
Generated adapters enforce the same restricted RPC contract as packaged apps.

Vite owns watching, module transforms, invalidation, and HMR/reload. Move the
existing listener-registry invalidation with the event entrypoint so edits to
`src/tokamak.ts` or its imports replace listeners without duplicates. Module
re-evaluation does not fire app startup. Native registrations remain independent
of JS reload, and native code changes require the existing rebuild path.

Development event handling depends on the host dev server and relay being
available, as it does today. Disconnection reports dispatch/call failure and
settles any pending native completion under its existing deadline/error
contract. Reconnection restores the session; it does not silently replay
completed or failed events. Packaged apps run the Tokamak worker on-device.

Validate the relocated development paths on devices. Prove that:

- Foreground calls and fresh background events reach the correct device's
  Tokamak worker and can call another plugin during handling.
- Main-worker bindings and frontend clients both work with the framework dev
  server, without leaking a native transport into the main worker.
- Vite HMR/reload replaces user handlers without duplicate registrations or
  stale native callbacks. Device disconnects and relay failure surface as errors.
- Background completion and any required response reach native code before
  the OS deadline. A disconnected dev server must not hold an OS callback open.

## Performance and implementation boundaries

Implement three improvements in the native bridge:

1. **Opaque payloads through the bridge.** Use transport metadata for routing,
   failure status, and completion. Encode and decode payloads only at the SDK/native
   endpoints, removing intermediate payload parsing and string conversions.
2. **Dispatch only to the threads an operation requires.** Honour each native
   API's thread or executor requirements. Calls and replies must not take an
   unconditional detour through the UI thread. Schedule JavaScript delivery on
   its owning runtime; use the main thread for native operations that require
   it.
3. **Keep large native data in native memory.** Pass runtime-owned native
   references between compatible plugins. For example, a camera frame passed
   to an encoder should not be copied into JavaScript and back. Carry opaque
   handles through the bridge while the data stays native. Expose bytes when
   JavaScript needs them, with explicit buffer ownership and lifetime, and
   minimise copies required for safe transfer.

Keep stream queues bounded, with overflow behaviour appropriate to the source.
Measure encoding work, thread hops, buffer copies, latency, and throughput on
actual targets.

The previous plan's exact C ABI, Rust object class, frame header, WebSocket
protocol, named-object scheme, ring buffer, and run-loop optimisations are
implementation candidates, not settled requirements. The single bytes-based
native transport contract is agreed; its framing and implementation remain
open. Choose those details only where the service contract and measurements
justify them. Do not carry forward the unmeasured latency, zero-copy, or
sensor-throughput claims as facts.

## Implementation sequence and proof

1. **Build the service-binding/RPC subsystem.** Integrate both bundles into the
   existing project build. Implement a service registry with per-service
   bundles/bindings, default/named entrypoints, `ServiceStub.fetch`, and public
   method dispatch. Implement cross-QuickJS value/error serialization, stream
   ownership/flow control, and invocation completion/cancellation for the
   initial subset. Add `TokamakEntrypoint` and its native context, keeping
   `event` reserved for existing host dispatch. Implement generator rejection
   rules and matching dev/frontend adapters. Prove bidirectional and nested
   calls with two actual QuickJS runtimes before migrating plugins; passing
   Miniflare tests does not satisfy this milestone.
2. **Move native connectivity and user code.** Route plugins, lifecycle, and
   native events exclusively to the Tokamak worker. Move `src/tokamak.ts` and
   the existing event-entrypoint loading and dispatch machinery out of the
   main worker into the reserved `event` path. Implement the shared bytes-based
   native transport with metadata separate from opaque payloads, encoding at
   the SDK/native endpoints, and dispatch that honours native thread requirements.
   Make native SDKs require the invocation's context. Remove the main worker's
   direct native transport and bootstrap state once migrated paths work.
3. **Establish SDK generation.** Generate thin typed frontend clients and
   service types. Keep platform SDKs and cross-platform native SDKs inside
   the Tokamak worker. Add optional browser fallbacks to frontend clients.
   Deliver the plugin scaffold, example app, and standard codecs alongside
   generation so authors do not have to assemble the transport layers.
4. **Migrate event and resource handling.** Preserve existing event completion,
   `waitUntil`, deadlines, error reporting, typed replies, and cancellation while
   adapting subscription ownership and background restoration. Add native
   references and buffer ownership so large data can remain native. Add WebView
   control through `ctx.webview` and main-worker calls through its service binding.
5. **Prove the model with real plugins.** Exercise location plus secure storage:
   a native background location event invokes user code, reads a credential,
   uploads data, and completes without a page. Also test foreground SDK calls,
   a browser location fallback, and a native-only method without a fallback.
   Create a small plugin outside the Tokamak repository using only the public
   authoring workflow, and verify local development, packaging, and publication.
6. **Measure and migrate.** Measure packaged and dev bridge encoding, thread
   hops, copies, call latency, stream behaviour, and resource cleanup. Migrate
   remaining plugins and their documentation, then remove superseded paths.

Keep changes reviewable and existing apps working through the migration.
Compatibility shims are temporary migration tools with explicit removal points,
not a second permanent native transport in the main worker.

Acceptance checks must cover:

- A new author can scaffold a plugin outside the Tokamak repository, implement
  a small capability, install it as a workspace dependency in an example app,
  test in `tok dev` and a packaged app, and publish/install it through normal
  npm tooling using only public packages and documentation.
- A one-platform plugin works without a browser fallback or cross-platform
  wrapper. A plugin composing existing bindings needs no native source.
- Adding a typed operation generates its frontend client, service wiring,
  native dispatch where applicable, and types without handwritten transport
  wrappers, duplicate signatures, or serializers for standard payloads.
- Packaged tests dispatch HTTP and public methods between two actual QuickJS
  runtimes, select default/named entrypoints, and supply only the receiving
  service's bindings/context. Exercise calls in both directions and nested
  calls without deadlocks or shared request state.
- Value conformance tests cover the complete allowlist: cycles/aliasing,
  `undefined`, non-finite numbers, `bigint`, containers, binary views, and
  reconstructed error fields. Mutating a received value does not mutate its
  source. Errors and unsupported values reject rather than hanging a call.
- Readable byte streams work as arguments/results without full buffering;
  ownership, backpressure, cancellation, EOF, producer errors, caller teardown,
  `waitUntil`, and deadline expiry release the appropriate runtimes/resources.
- Generator negative cases reject callback parameters, function results,
  `RpcTarget`/stubs, unsupported stream kinds, and unbounded types. Runtime
  cases catch unsupported values hidden by JavaScript or type assertions.
  Run the same contract cases against dev bindings and generated frontend
  clients so Miniflare cannot conceal a packaged-runtime incompatibility.
- Third-party dependencies and shared application modules imported from
  `src/tokamak.ts` resolve through the existing project configuration in both
  builds and local development, without a separate project or manual wiring.
- Main-worker and page bundles receive no injected native bridge, native
  bootstrap state, or device relay credentials; web builds omit the Tokamak
  worker. Generated clients and service bindings do not bundle native SDKs.
- Native SDK calls type-check with `TokamakContext`, including in shared
  helpers, and reject an ordinary worker context or missing context. Importing
  an SDK alone neither opens a connection nor grants native access.
- HTTP, RPC, and event handlers receive their own host-supplied native context;
  callers supply only application data. `event` is absent from the public RPC
  and generated frontend interfaces and cannot be dispatched through them.
- Structured and binary plugin payloads use the same native transport in
  packaged apps and local development. Only the endpoints interpret payloads;
  raw binary data requires no JSON/base64 conversion.
- Routing, success/failure status, and completion use transport metadata
  without parsing payloads in intermediate layers.
- Calls and replies respect native API and JavaScript runtime thread
  requirements without an unconditional UI-thread hop.
- Passing a native buffer between compatible plugins transfers its handle
  without copying its contents into JavaScript. Reading bytes, releasing the
  handle, cancellation, and invocation teardown honour buffer ownership and
  lifetime. Measure the copies on paths that expose bytes to JavaScript.
- Generated types match exposed methods and fallback signatures. Native-only
  methods remain callable in native pages and fail clearly on the web. Editor
  support annotations and capability flags agree with the selected target;
  permission denial is not misreported as missing implementation support.
- Native SDK failures propagate; they do not trigger a browser fallback.
- The migrated event dispatcher invokes the Tokamak worker for lifecycle and
  plugin events, including when no invocation is active and no page is present,
  preserving existing listener-awaiting, `waitUntil`, deadline, error-reporting,
  and response-selection semantics, including completion without a reply.
- A page reload releases its resources while app-owned background work remains
  registered. Restart follows each plugin's declared restoration behaviour;
  registering the same stable ID does not duplicate native registrations.
- Synchronous OS callbacks answer from native configuration/defaults without
  waiting for JavaScript; any separately supported asynchronous completion
  still uses the existing event result/deadline contract.
- `ctx.webview` navigation, current-page queries, and messaging work from user
  Tokamak code, as do main-worker service calls. Outcomes are explicit when no
  page is loaded or the view/service is unavailable. Missing pages neither open
  implicitly nor accumulate an implicit durable message queue.
- Device/simulator tests establish target-specific behaviour; host-only tests
  do not prove background operation. Exercise physical-device suspension where
  needed, separately from continuing work after the app leaves the foreground.
- The same workflows work in local development through the second Vite-hosted
  Worker, including the existing HMR/reload flow. Dev-server/relay loss produces
  clear failures and completes pending native callbacks within their deadlines.

## Decisions

These resolve the seven previously open architectural decisions:

1. **Packages:** one package per plugin, with a native root, platform subpaths,
   `/client`, and `/events`; core controls remain in `@tokamakdev/tok`.
2. **Generation and support:** derive contracts from TypeScript exports;
   generate routes, clients, editor support annotations, and capability flags
   using the existing plugin manifest.
3. **Routing:** build a new service subsystem with the explicit value/error/
   readable-byte-stream RPC subset, enforced in packaged apps, dev, and clients.
4. **Ownership:** invocation-scoped resources by default; explicit native-owned
   registrations with stable IDs for background work and documented restoration.
5. **Synchronous callbacks:** native defaults or previously supplied typed
   configuration answer immediately; asynchronous event replies cannot override them.
6. **App control:** expose WebView operations on `ctx.webview`; use ordinary
   main-worker bindings and add host controls only for concrete requirements.
7. **Development:** use a second Vite-hosted Worker with the existing native
   relay, event dispatch, and Vite HMR/reload model.

## Related documentation

- `../tokamak/plugins/GUIDELINES.md`: update plugin authoring conventions to
  match this plan during implementation.
- `../tokamak/docs/plans/tokamak-plugins.md`: describes the earlier plugin
  model; this document supersedes conflicting architectural decisions.
- `../tokamak/docs/plans/dev-mode.md` and `vite.md`: reconcile native module
  placement, generated worker bundles, and relay ownership with this plan.
- [Cloudflare service bindings](https://developers.cloudflare.com/workers/runtime-apis/bindings/service-bindings/):
  the Worker-to-Worker HTTP/RPC model to implement on native.
- [Cloudflare RPC entrypoints](https://developers.cloudflare.com/workers/runtime-apis/bindings/service-bindings/rpc/):
  class methods with receiving-worker context and bindings on `this`.
- [Cloudflare RPC values and streams](https://developers.cloudflare.com/workers/runtime-apis/rpc/):
  reference for the explicitly limited contract above, not a full-compatibility claim.
- [Cloudflare RPC errors](https://developers.cloudflare.com/workers/runtime-apis/rpc/error-handling/):
  transported error data without remote class identity or stack guarantees.
- [Cloudflare RPC lifecycle](https://developers.cloudflare.com/workers/runtime-apis/rpc/lifecycle/):
  invocation ownership and cancellation; the initial subset omits remote-object leases.
- [Cloudflare RPC visibility](https://developers.cloudflare.com/workers/runtime-apis/rpc/visibility/):
  public prototype methods and private implementation helpers.
- [Cloudflare RPC TypeScript](https://developers.cloudflare.com/workers/runtime-apis/rpc/typescript/):
  generated service types from entrypoint classes.
- [Cloudflare Vite programmatic configuration](https://developers.cloudflare.com/workers/vite-plugin/reference/programmatic-configuration/):
  generated auxiliary-worker configuration and service bindings for local development.
- [Android JobService](https://developer.android.com/reference/android/app/job/JobService):
  immediate continuation response and separate asynchronous job completion.
