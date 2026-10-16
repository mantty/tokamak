# WebAssembly spike

Packaged Workers can import Wasm with the same interface as Cloudflare Workers:

```js
import module from "./compute.wasm";

const instance = await WebAssembly.instantiate(module, {
  env: { report: value => console.log(value) },
});

export default {
  fetch() {
    return new Response(String(instance.exports.add(20, 22)));
  },
};
```

Wrangler's `CompiledWasm` rules identify modules. Tokamak copies their bytes into
the package and imports them as `WebAssembly.Module` objects. JavaScript runs in
QuickJS; Wasm runs synchronously in WAMR's fast interpreter. Local development
continues to use workerd's WebAssembly implementation.

Supported: module imports/exports/custom sections, synchronous `Instance` and
asynchronous `instantiate(module, imports)`, numeric function imports/exports,
`i64` as `BigInt`, multiple results, start functions, exported memory and globals,
`Memory` and numeric `Global` constructors, traps, and validation. Memory views
share the native allocation. JavaScript exceptions from imports preserve their
identity. Cancellation interrupts Wasm loops and startup functions.

Like Cloudflare Workers, compilation of application-provided bytes is disabled:
use a `.wasm` import instead of `compile(bytes)`, `new Module(bytes)`, or
`instantiate(bytes)`. Streaming compilation APIs are not exposed.

## Limits of the spike

WAMR's C API does not link imported memories, tables, or mutable globals with
JavaScript semantics. Tokamak rejects these imports with `LinkError`. Tables
within Wasm execute, but their JavaScript methods are not implemented. Reference
values cannot cross the JavaScript boundary. SIMD, threads/shared memory,
exception handling, GC, memory64, and WASI are not enabled. This is not complete
WebAssembly specification parity with V8.

Each instance currently reloads its module into a separate store. This keeps
imports and lifetime ownership independent; there is no compiled-module cache.
The implementation adds no plugin APIs or separate JavaScript environment.

## Linking

The interpreter is a separately linked runtime component, `tokamak_wasm`.
Packaged Workers currently retain it. A Worker can use `WebAssembly.validate`
or construct memory without importing a `.wasm` file, so imported-module metadata
alone cannot reliably prove that the component is unused. Automatic omission
would require inspecting application code or an explicit application declaration.
Development shells omit it because their Worker runs in workerd.

The macOS arm64 example executable grows by approximately 161 KiB with the
component linked. This measures the stripped executable, including the Rust
bridge and interpreter, rather than the size of the static library.

## Contributor checks

```sh
cargo test -p tokamak --features native --lib runtime_contract_tests::wasm
cargo test -p tokamak-cli worker::tests
cargo test -p tokamak-cli pipeline::tests
```

The first command requires the existing Miniflare fixture dependencies in
`tokamak/tests/quickjs_runtime`. It runs the same Worker in WAMR/QuickJS and
workerd, plus cancellation and unsupported-import checks.

The spike's validation covers the packaged macOS arm64 app and its WebView,
Vite local development, the native runtime and CLI suites, a Linux arm64 C bridge
execution, and Cargo builds of the WAMR dependency for Android arm64 and iOS
device/simulator arm64. Windows execution and mobile device execution remain
unverified.

See [the embedding notes](../wamr-sys/README.md) for vendoring and upstream changes.
