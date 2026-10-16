# WebAssembly

Packaged Workers import Wasm with the Cloudflare Workers interface:

```js
import module from "./compute.wasm";

const memory = new WebAssembly.Memory({ initial: 1, maximum: 4 });
const counter = new WebAssembly.Global({ value: "i32", mutable: true }, 0);
const table = new WebAssembly.Table({ element: "anyfunc", initial: 1, maximum: 8 });
const instance = await WebAssembly.instantiate(module, {
  env: { memory, counter, table },
});

export default {
  fetch() {
    return new Response(String(instance.exports.run()));
  },
};
```

The import names and types must match the module. Wrangler's `CompiledWasm`
rules identify modules; Tokamak packages their bytes as `WebAssembly.Module`
imports. JavaScript runs in QuickJS and Wasm runs synchronously in WAMR's fast
interpreter. Local development uses workerd's WebAssembly implementation.

## Supported

- Module import/export metadata and custom sections, `Instance`,
  `instantiate(module, imports)`, and `validate`.
- Numeric functions, `i64` as `BigInt`, multiple results, JavaScript callbacks,
  start functions, traps, and execution cancellation.
- Imported and exported memories with shared contents and object identity.
  Successful growth detaches old buffers, including zero-page growth and growth
  before a callback into JavaScript.
- Imported and exported tables, `Table` construction, `get`, `set`, `grow`,
  and `length`; function references can cross instances.
- Imported and exported globals, including mutable globals and reference values.
  JavaScript and Wasm observe each other's writes.
- `externref` and `funcref` across calls, globals, and tables. JavaScript table
  and global descriptors use `anyfunc` for function references.
- Fixed-width SIMD, bulk memory operations, tail calls, extended constant
  expressions, sign extension, and non-trapping float-to-integer conversions.

SIMD instructions execute inside Wasm. As in V8, JavaScript cannot pass or
receive a `v128` value directly. JavaScript exceptions from callbacks retain
identity, including callbacks and reentry across instances. Functions exposed
through imported tables remain callable if a later instantiation step traps.

Like Cloudflare Workers, compilation of application-provided bytes is disabled:
use a `.wasm` import instead of `compile(bytes)`, `new Module(bytes)`, or
`instantiate(bytes)`. Streaming compilation APIs are not exposed.

## Remaining Cloudflare compatibility gaps

The fixture's workerd runtime supports the following features; Tokamak does not:

| Feature | Affected code |
| --- | --- |
| Shared memories and atomics | `shared: true`, Wasm thread instructions |
| Memory64 and table64 | 64-bit addresses and `address: "i64"` descriptors |
| Multiple memories | Modules declaring more than one memory |
| Wasm GC and typed function references | Wasm-managed structs/arrays and typed reference instructions |
| Exception handling | Wasm tags, throw/catch instructions, `Tag`, `JSTag`, and `Exception` |
| Relaxed SIMD | Relaxed SIMD opcodes; ordinary fixed-width SIMD is supported |
| JavaScript promise integration | `Suspending`, `promising`, and suspension APIs |
| Resizable memory buffers | `Memory.toResizableBuffer()` and `toFixedLengthBuffer()` |

This is not full V8/WebAssembly parity. The feature audit compares validation and
API availability with the pinned Miniflare/workerd fixture. Differential execution
tests cover the supported imports, calls, resource sharing, and SIMD behavior.

The embedding keeps instances, wrappers, and external references until their
Worker JavaScript context ends. It does not collect individual Wasm instances or
references within that context. Each instantiation reloads the module; there is
no compiled-module cache. There is no built-in WASI host, JIT, or AOT execution.

## Linking

The interpreter is a separately linked runtime component, `tokamak_wasm`.
Packaged Workers retain it: a Worker can use `WebAssembly.validate` or construct
memory without importing a `.wasm` file, so imported-module metadata cannot
reliably prove that the component is unused. Automatic omission requires code
inspection or an explicit application declaration. Development shells omit it
because their Worker runs in workerd.

## Contributor checks

```sh
cargo test -p tokamak --features native --lib
cargo clippy -p tokamak --features native --lib --tests -- -D warnings
```

The Wasm tests require the Miniflare dependencies in
`tokamak/tests/quickjs_runtime`. They compare identical Worker code against
workerd, test interruption, and audit additional feature compatibility.

See [the embedding notes](../wamr-sys/README.md) for ownership, vendoring, and
upstream patches.
