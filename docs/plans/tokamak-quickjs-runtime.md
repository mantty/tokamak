# tokamak QuickJS runtime

Status: draft

Supersedes: [request-scoped Bare realms and VFS](tokamak-request-realms-vfs.md)

Contract: [Workers JavaScript compatibility](tokamak-workers-node-compat.md)

## Objective

Replace Bare with a tokamak-owned Rust runtime embedding QuickJS-NG. Execute each
Worker request in a fresh QuickJS runtime with request-owned native resources
and a request-owned virtual filesystem.

Keep the application contract unchanged: Wrangler input, Cloudflare Workers
and current Node compatibility, the public tokamak runtime API, native shells,
the stable HTTPS origin, and client-certificate authentication.

## Scope

The runtime must preserve:

- request and response streaming;
- `waitUntil()` and WebSocket event lifetimes;
- outbound fetch, DNS, TCP, TLS, and WebSockets;
- assets and persistent Cache Storage;
- suspend, resume, certificate renewal, and shutdown; and
- the existing macOS, iOS, Android, and Windows packaging flows.

It must add Workers-compatible `node:fs` with request-scoped `/tmp`.

It does not add Cloudflare services, a general Node runtime, WebAssembly, or
features deferred by the compatibility contract.

## Design decisions

- Use a published `rquickjs` release and its bundled QuickJS-NG sources. Carry
  no tokamak engine or binding patches.
- Implement ECMA-402 over pinned ICU4X crates and one shared immutable CLDR
  data image.
- Statically link QuickJS-NG and tokamak's native modules into each target.
- Use one `JSRuntime` and one `JSContext` per request.
- Compile the Worker to QuickJS bytecode during `tok build` and load that
  bytecode per request.
- Use the same pinned `rquickjs` and QuickJS-NG revision in `tokamak-cli` and the
  target runtime. Platform packs record `tokamakVersion`; the CLI accepts a pack
  only when that value exactly matches the CLI's package version.
- Schedule each request on the shared Tokio runtime. Keep its QuickJS runtime,
  context, and JavaScript execution on one Tokio blocking task.
- Keep asynchronous host I/O on the same Tokio runtime so completions can be
  routed to the request that owns them.
- Keep protocol handling, request preparation, body transport, response
  delivery, backpressure, cancellation, and native resource work in Rust.
- Implement public Web, Node, and Cloudflare API shape in JavaScript.
- Keep one builtin-module registry and one implementation of each feature.
- Do not impose a tokamak request-count cap or return `503` because a tokamak
  queue is full. Let Tokio and the operating system apply resource
  backpressure, while tokamak accounts for memory and cancels work cleanly.
- Keep no idle or pre-warmed runtimes until profiling proves they are needed.
- Do not add a JavaScript-engine abstraction, a production dual-backend mode,
  N-API, dynamic native modules, or a third-party add-on ABI.

App users consume a prebuilt tokamak CLI and platform packs. QuickJS-NG, `qjsc`,
Rust, a C compiler, and bindgen require no separate user installation.

## Architecture

```text
WebView
  <-> Rust CONNECT/TLS/HTTP/WebSocket gateway and host I/O
  <-> Tokio scheduler
        `- RequestExecution
             |- QuickJS runtime and context
             |- RequestState
             |    |- native resource table
             |    |- /tmp VFS
             |    |- memory accounting
             |    `- cancellation and lifetime state
             `- shared immutable WorkerImage and /bundle
```

`tokamak` continues to own certificates, application lifecycle, events,
and the shell-facing API.

The `tokamak/src/quickjs` module owns the engine, runtime threads, gateway, Worker
image, request lifecycle, and asynchronous host I/O. Capability modules such as
`fs`, `streams`, `network`, `events`, `globals`, and `builtins` own their API
implementations and source files. It
replaces `tokamak-bare` directly; no backend trait is added.

### Rust request fast path

Rust owns a request until Worker JavaScript must run. The gateway parses the
protocol, enforces limits, prepares request metadata, and creates native body,
resource, and cancellation handles. QuickJS receives a small request
descriptor and opaque handles rather than copied bodies or protocol work.

After JavaScript dispatch, Rust owns response framing, body streaming,
backpressure, cancellation, socket I/O, and teardown. Crypto, compression,
encoding, VFS, cache, assets, and other data-heavy operations use coarse native
calls. JavaScript owns Worker decisions and public compatibility wrappers.
Differential tests ensure this division preserves workerd-observable behavior.

### Engine boundary

Pin one published `rquickjs` release and its bundled QuickJS-NG revision.
Disable dynamic loading and experimental parallel execution. Use the
high-level `rquickjs` API for ordinary engine access.

Keep tokamak's explicit unsafe code inside the engine module. It implements the
charging allocator and trusted bytecode loading. Do not use the raw `qjs` API
elsewhere. Never load QuickJS bytecode supplied by an app or external source.

Use `rquickjs` for runtime creation and destruction, modules, native functions
and classes, promises, retained values, job execution, rejection tracking,
interrupts, memory accounting, and diagnostics. Do not install QuickJS's
`std` or `os` modules.

A complete `JSRuntime` is the isolation boundary. QuickJS runtimes have
independent heaps and cannot exchange JavaScript objects. Shared data is
immutable Rust data or an app-scoped Rust service behind a native binding.

### Worker image

The packer emits one self-contained ESM Worker. Esbuild includes application
code, ordinary dependencies, and the JavaScript compatibility layer. The only
external module is tokamak's statically linked `tokamak:host` module. Every other
unresolved import is a build error.

The packaged app contains:

- Worker bytecode and its source map;
- normalized Worker environment data;
- a manifest and files for read-only `/bundle`; and
- the existing static asset manifest and files.

`tokamak-cli` statically links QuickJS-NG through `rquickjs`. After esbuild emits
the bundled ESM, `tokamak-cli` uses `Module::declare()` and `Module::write()` to
compile and serialize it without evaluation. Bytecode endianness follows the
target; all current targets are little-endian. `qjsc` is not packaged or
invoked; `rquickjs` provides its compile-and-serialize operation in-process.

At startup, tokamak loads an immutable `WorkerImage` containing the packaged
bytecode, source map, and `/bundle` data. It is charged once to the app-wide
memory budget and shared by all requests as bytes, never as JavaScript values.

Each request runtime loads and evaluates the bytecode, so globals, module
state, top-level side effects, and intrinsic objects are request-local. Module
initialization therefore runs once per request. Workers does not guarantee
module-state persistence between requests.

### Threads and asynchronous work

The shared Tokio multi-thread runtime runs each request in one blocking task.
Each request keeps its QuickJS runtime, context, and request state there;
independent requests therefore run in parallel without sharing JavaScript
state. The gateway is currently blocking and can move to Tokio async I/O
without changing request ownership.

A native async operation records its execution and resource IDs. The request
owner retains the promise settlement functions and receives only plain Rust
data from Tokio. A completion is delivered to that request owner, which
settles the promise and drains only that runtime's job queue. A completion for
a disposed execution is dropped. No other thread calls QuickJS.

Synchronous JavaScript runs to completion on its request worker and does not
serialize unrelated requests. The QuickJS interrupt handler reads
cancellation and shutdown flags set by Tokio or the shell thread, allowing
them to pre-empt long-running JavaScript.

`suspend()` stops new dispatch, closes active gateway connections, and lets the
current JavaScript turn finish. WebSocket executions end at their next event
boundary; other executions drain normally. New work waits without consuming a
polling loop. `resume()` refreshes certificates before dispatch. Shutdown
cancels all executions, closes the gateway, and joins the scheduler.

Platform shells do not call `suspend()` for ordinary focus or background
transitions. Desktop shells keep the runtime running. Mobile shells allow the
operating system to pause the process and probe the gateway on foreground,
reconfiguring and reloading the WebView only when the listener moved ports.

### Native modules

Register one statically linked `tokamak:host` ES module assembled from focused
Rust modules:

| Area | Native responsibility |
| --- | --- |
| Execution | logging, clocks, timers, cancellation, and promise completion |
| Transport | body streams, fetch, DNS, TCP/TLS, and WebSockets |
| Storage | request VFS, Cache Storage, static assets, and `/bundle` |
| Algorithms | secure randomness, cryptography, compression, encoding, and ICU4X internationalization |

The native module exposes primitives rather than public Web or Node objects.
JavaScript builds lightweight `Request`, `Response`, streams, Web Crypto, Node
modules, and Cloudflare module wrappers over native data and operations. APIs
that need the same operation share one native primitive.

Native wrappers carry opaque resource IDs into the owning request's resource
table. They never expose Rust pointers. Explicit close and cancellation are
the normal lifecycle; QuickJS finalizers only release abandoned wrappers.
`tokamak:host` is an internal API, not a security boundary, so every operation
still validates its capability and inputs.

### Request lifecycle

```text
queued -> starting -> running -> responding/draining -> disposed
                         `-> cancelled/error ---------> disposed
```

Every accepted request gets one Tokio blocking task. There is no tokamak request
queue with a fixed size and no intentional `503` response for concurrency.
Tokio's blocking scheduler, memory accounting, and the operating system still
provide real resource backpressure. Requests never share a runtime, global
object, native handle, or VFS.

An execution remains alive while JavaScript can still be invoked by:

- the handler's returned promise;
- a JavaScript-backed response body;
- registered `waitUntil()` work; or
- an accepted WebSocket and its handlers.

A client disconnect cancels its response stream but not registered
`waitUntil()` work. Incoming bodies and native resources are cancelled once no
tracked JavaScript owner can use them. Untracked floating work does not extend
the event lifetime.

Every completion path uses the same explicit teardown:

1. reject new native work and mark the execution cancelled;
2. detach pending completions;
3. cancel timers, streams, sockets, fetches, and other native resources;
4. release all retained QuickJS values;
5. destroy the context and runtime; and
6. drop `RequestState`, including its resource table and `/tmp`.

The order is mandatory because `rquickjs` retained values must not outlive
their runtime. Teardown is idempotent and never waits for JavaScript garbage
collection.

### Virtual filesystem

One Rust VFS backs `node:fs`, `node:fs/promises`, and callback forms:

- `/bundle` is an immutable view of the shared `WorkerImage`;
- `/tmp` is a mutable in-memory tree owned by one `RequestState`;
- `/dev/null`, `/dev/random`, `/dev/full`, and `/dev/zero` implement the
  Workers device contract; and
- no path can reach host storage, Cache Storage, or another request.

The VFS implements Workers path normalization, symlinks, descriptors, errors,
4,096-character paths, 48 path segments, 128 MB files, epoch timestamps, and
the documented unsupported watch, glob, permission, and ownership operations.

All API forms call one synchronous filesystem core. Callback and promise
delivery order is fixed by differential tests. The VFS therefore needs no
async request-context propagation.

QuickJS uses a custom allocator charged to one app-wide budget. `/bundle`,
`/tmp`, and tokamak-owned body and stream buffers use the same budget. Bounded
queues and resource-count limits constrain allocations internal to HTTP, TLS,
and WebSocket libraries. Budget exhaustion returns the API's normal
allocation or storage error instead of aborting the process. Diagnostics
report each category separately.

### Gateway and network I/O

The Rust gateway preserves the current protocol:

- bind only to loopback;
- accept `CONNECT` only for the configured stable host;
- perform server TLS with mandatory tokamak client authentication;
- serve streaming HTTP and WebSocket upgrades; and
- report the bound port synchronously from `Runtime::start()`.

Use maintained Rust protocol and TLS libraries with bounded queues. Preserve
backpressure, cancellation, repeated `Set-Cookie` headers, HEAD responses, and
disconnect handling. Do not implement general protocol parsers.

`tokamak` remains the certificate authority. It gives the gateway each
renewed TLS configuration before another handshake is accepted.

Outbound fetch, Node HTTP clients, `cloudflare:sockets`, DNS, WebSockets, and
EventSource share one request-owned network layer and address policy. Assets
and persistent Cache Storage remain app-scoped Rust services and do not live
in `/tmp`.

### JavaScript compatibility layer

Move engine-neutral modules from `bare/runtime` into an engine-neutral runtime
package. Keep existing modules whose behaviour passes the compatibility tests;
replace their `bare-*` dependency with an `tokamak:host` primitive or a standard
JavaScript implementation.

Keep public API shape and JavaScript-specific semantics in JavaScript. Keep
protocol work, data movement, and native capabilities in the corresponding
Rust module. Do not maintain parallel Bare and QuickJS versions of a feature.

The builtin registry remains the only source for esbuild aliases, functional
Node modules, Workers stubs, Wrangler shims, and
`process.getBuiltinModule()`. Direct `bare-*` imports become build errors.

QuickJS-NG does not implement ECMA-402 `Intl`. Implement the constructors,
prototypes, and locale-sensitive built-in methods in JavaScript over focused
ICU4X primitives in `tokamak:host`. Store CLDR data once as immutable app-wide
data; request runtimes contain only their JavaScript wrappers and formatter
handles.

The baseline inventory fixes the exact API and option matrix against workerd
and the current runtime. ECMA-402 Test262 tests and differential fixtures cover
it. Binary size, data size, per-runtime formatter memory, and startup time are
an early feasibility gate. The runtime cannot replace Bare until those tests
pass without application changes.

Meaningful `AsyncLocalStorage` remains separate compatibility work. VFS
ownership does not depend on it because each runtime has one `RequestState`.

## Build changes

| Area | Change |
| --- | --- |
| Top-level `bare/` | Delete it; do not add a corresponding `quickjs/` directory because Cargo builds QuickJS-NG through `rquickjs`. |
| `tokamak/src/quickjs` | Keep the QuickJS engine and gateway as an `tokamak` module. |
| `tokamak` | Own the runtime, QuickJS, and capability modules in Rust and JavaScript. |
| `tokamak-cli` | Compile bundled ESM to bytecode and package native apps. |
| Root-level Worker-package modules | Package Worker bytecode, source map, environment, and `/bundle` manifest. |
| Runtime JavaScript | Remove all `bare-*` dependencies as features move. |
| Worker packer | Emit bundled ESM; remove the CommonJS worklet and `bare-pack`. |
| `tools/xtask` | Build platform packs and write their tokamak version. |
| Native shells | Keep the ABI and co-locate each platform build entrypoint with its shell. The CLI invokes the fixed platform-pack entrypoint convention. |
| CI | Build the pinned engine for every target and package only prebuilt artifacts for users. |

## Implementation sequence

### 1. Fix the baseline

1. Inventory every current global, builtin, Cloudflare module, direct and
   transitive `bare-*` dependency, and native capability. Give each one a
   compatibility fixture and a JavaScript or Rust owner.
2. Run compatibility fixtures independently of the Bare test runner.
3. Add fixtures for module initialization, `Intl`, streams, `waitUntil()`,
   WebSockets, cancellation, assets, cache, suspend/resume, and VFS semantics.
4. Record startup, idle memory, latency, throughput, and shutdown for the
   Astro example on macOS, iOS Simulator, and Android emulator.

### 2. Prove QuickJS before migration

1. Add `tokamak/src/quickjs` with pinned published `rquickjs` and ICU4X releases.
2. Prove native functions and classes, async promises, ESM, top-level await,
   compile-without-evaluation, bytecode reuse, source-mapped errors, pending
   jobs, rejection tracking, interrupts, charging-allocation failure,
   finalizers, cancellation, and teardown under sanitizers where available.
3. Build and run that fixture on macOS arm64/x64, iOS device and simulators,
   Android arm64, and Windows x64. Generate mobile bindings without patches.
4. Create and destroy 10,000 sequential runtimes and exercise 100 concurrent
   runtimes. Logical counts must return to zero and equal batches must not show
   continuing RSS growth.
5. Profile runtime creation, bytecode loading, module initialization, teardown,
   per-request memory, peak concurrency, engine-boundary calls and copied
   bytes, time spent in Rust and JavaScript, engine size, and `Intl` data size
   on real bundles.
6. Build the ICU4X-backed `Intl` proof and pass the selected ECMA-402 Test262
   and differential fixtures before porting the broader API surface.

Stop if any supported target, deterministic teardown, bytecode reuse, `Intl`,
or the measured mobile memory envelope fails.

### 3. Build one end-to-end slice

1. Add build-time bytecode compilation and Worker image loading.
2. Add the Tokio scheduler, request-owned QuickJS execution, and the
   CONNECT/TLS/HTTP gateway.
3. Install only dispatch, console, timers, errors, request/response, and body
   streaming.
4. Start the packaged Astro app through the real mTLS gateway on macOS, iOS
   Simulator, and Android emulator.

### 4. Add request ownership and VFS

1. Add `RequestExecution`, `RequestState`, resource ownership, cancellation,
   and the unified teardown path.
2. Implement exact response-stream, `waitUntil()`, WebSocket, disconnect,
   suspend/resume, and shutdown lifetimes.
3. Add the VFS, then expose it through `node:fs` and `node:fs/promises`.
4. Stress every normal and abnormal completion path and verify that its
   runtime, resources, and VFS are gone.

### 5. Port compatibility in dependency order

1. Encoding, events, buffers, URL, path, assertions, utilities, process, and
   the builtin registry.
2. Web Streams and Node streams.
3. Fetch, HTTP adapters, forms, blobs, and EventSource.
4. Web Crypto, Node crypto, compression, and `Intl`.
5. DNS, TCP/TLS, `cloudflare:sockets`, and WebSockets.
6. Cache Storage, assets, MessageChannel, scheduler, performance,
   `cloudflare:workers`, Workers stubs, and Wrangler shims.

A feature moves once, with its differential tests. Remove its final `bare-*`
dependency at the same time.

### 6. Integrate and cut over

1. Update app layout, CLI packing, platform-pack manifest contract, artifacts,
   and licenses.
2. Build and start the Astro example through the normal package scripts on
   macOS, iOS Simulator, and Android emulator.
3. Build physical iOS, both simulator architectures, both macOS
   architectures, Android arm64, and Windows x64 platform packs.
4. Switch `tokamak` directly to `tokamak/src/quickjs`.
5. Delete the top-level `bare/` directory, `tokamak-bare`, BareKit, `bare-pack`,
   Bare packaging, and unused `bare-*` dependencies. Do not add a top-level
   QuickJS build directory.
6. Run every test and platform build again after Bare is absent.

## Verification

- Differential fixtures compare tokamak with pinned workerd/Miniflare behaviour,
  including values, errors, headers, streams, cancellation, and lifetimes.
- Bytecode produced by each supported host CLI loads on every target available
  from that host. Platform packs whose `tokamakVersion` differs from the running
  CLI's package version fail during build.
- Every builtin is tested through static import, its supported unprefixed
  form, and `process.getBuiltinModule()`.
- Concurrent requests cannot observe each other's globals, module state,
  environment, handles, timers, or `/tmp`.
- `/tmp` survives through tracked response, `waitUntil()`, and WebSocket work,
  then disappears after normal completion, errors, cancellation, and timeout.
- Late native completions cannot invoke or retain a disposed runtime.
- Streaming tests cover bounded buffering, backpressure, body locking,
  cloning, cancellation, and disconnects.
- Gateway tests cover CONNECT routing, mTLS, certificate renewal, HTTP details,
  WebSocket close metadata, and the shared outbound address policy.
- VFS tests cover `/bundle`, `/tmp`, `/dev`, descriptors, symlinks, limits,
  errors, all API forms, and attempts to escape to host paths.
- Profiling reports p50, p95, and p99 startup, runtime creation, bytecode load,
  module initialization, first byte, and teardown, plus time spent in Rust and
  JavaScript, engine-boundary calls, copied bytes, logical memory counters, and
  RSS for sequential and 100-request concurrent runs.

## Acceptance criteria

- Every request uses a fresh QuickJS runtime, global object, module instance,
  resource table, and `/tmp` VFS.
- All normal and abnormal event lifetimes explicitly destroy their runtime and
  VFS without waiting for garbage collection.
- Ten thousand sequential requests return all logical request-owned counts to
  zero and show no continuing RSS growth across equal batches.
- Concurrent requests remain independent. There is no tokamak request-count cap;
  resource exhaustion is reported through normal cancellation or allocation
  errors rather than a synthetic concurrency `503`.
- The shared Worker image and `/bundle` are immutable and charged once.
- Request and response bodies remain Rust-owned streams unless JavaScript
  explicitly reads or materializes them.
- App-wide memory accounting covers QuickJS, VFS, and tokamak-owned buffers;
  queues and native resource counts are bounded.
- Current Workers and Node behaviour, including `Intl`, does not regress.
- The existing Astro app runs unchanged on macOS, iOS Simulator, and Android
  emulator using the normal package scripts.
- Every supported platform pack builds from pinned sources.
- The local mTLS gateway, streaming HTTP, WebSockets, assets, cache, suspend,
  resume, and shutdown retain their current behaviour.
- `node:fs` matches the Workers VFS contract and cannot access host storage.
- Every item in the baseline capability inventory has a final implementation
  or the same explicit unsupported behaviour as the compatibility contract.
- Bare, BareKit, `bare-pack`, `tokamak-bare`, and runtime `bare-*` dependencies
  are absent after cutover.
- App users install no Rust, QuickJS, `qjsc`, C compiler, or native add-on
  toolchain.

## References

- [QuickJS-NG C API](https://quickjs-ng.github.io/quickjs/developer-guide/intro/)
- [QuickJS-NG platforms](https://quickjs-ng.github.io/quickjs/supported_platforms/)
- [`rquickjs` documentation](https://docs.rs/rquickjs/latest/rquickjs/)
- [ICU4X Rust components](https://docs.rs/icu/latest/icu/)
- [Cloudflare Workers VFS](https://developers.cloudflare.com/workers/runtime-apis/nodejs/fs/)
- [Cloudflare Workers request context](https://developers.cloudflare.com/workers/runtime-apis/context/)
