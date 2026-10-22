# WAMR embedding

`vendor/` contains `core/`, `build-scripts/`, and `LICENSE` from
[WAMR 2.4.5](https://github.com/wasm-micro-runtime/wasm-micro-runtime/releases/tag/WAMR-2.4.5).
`simde/` contains the headers and license from
[SIMDe v0.8.2](https://github.com/simd-everywhere/simde/releases/tag/v0.8.2).
Cargo builds these sources without downloading build-time dependencies.

The fast interpreter uses software bounds checks and SIMDe for fixed-width SIMD.
It does not allocate executable memory. JIT, AOT, built-in WASI/libc imports,
thread creation and dynamic module discovery are disabled. Shared memory and
atomic instructions are enabled; Worker agents reject blocking waits.

The relaxed SIMD implementation comes from
[WAMR PR 4950](https://github.com/wasm-micro-runtime/wasm-micro-runtime/pull/4950),
pinned at `a169d8cd89663b0f647b4597285ea25e8f5dc74f`. The vendored changes cover
opcode definitions, validation, and fast-interpreter execution. Tokamak's CMake
configuration enables `WASM_ENABLE_RELAXED_SIMD`.

## Ownership and linking

The process owns one WAMR engine. Each Worker JavaScript context owns a native
object registry containing modules, instances, memories, tables, globals, and
functions. Registry IDs preserve identity across imports, exports, and function
references. Objects remain allocated until the context is released.

`bridge.c` uses WAMR's C API for loading and type metadata, and its interpreter
embedding API for linking and invocation. `tokamak.h` declares the engine hooks.
Rust uses opaque handles and tagged values. JavaScript wrappers retain externref
values and map them to numeric reference IDs.

Imported memories use the same native allocation. Imported globals point to the
same value storage. Tables have stable headers and separately growable element
arrays. Function references identify their owning instance; cross-instance calls
use that instance's invocation stack, including reentry through JavaScript.

External memory buffers retain the registry. Growth detaches fixed unshared buffers before JavaScript resumes. Shared
buffers retain their allocation; resizable buffers update existing views. Invocation state
exists before element segments can expose functions from a failed instantiation.

## Upstream changes

- `build-scripts/version.cmake` reads the pinned `core/version.h`.
- `build-scripts/runtime_lib.cmake` uses vendored SIMDe on every target.
- `interpreter/wasm.h` adds the embedding context, C API instantiation marker,
  and original import-section positions.
- `interpreter/wasm_loader.c` preserves import declaration order and permits
  host memory growth when no `memory.grow` instruction exists.
- `common/wasm_c_api.c` marks C API modules and runs start functions after linking.
- `interpreter/wasm_runtime.h` adds shared global storage, table element storage,
  function ownership, and memory growth generations.
- `interpreter/wasm_runtime.c` links imported resources through the registry,
  initializes canonical references, prepares invocation state before segments,
  and retains partially initialized instances. It skips automatic native startup
  exports for C API modules; JavaScript instantiation runs only the start section.
- `common/wasm_runtime_common.c` routes embedding callbacks through raw cells,
  preserving numeric reference IDs.
- `interpreter/wasm_interp_fast.c` accesses shared resources and canonical
  references, invokes foreign functions on their own stacks, copies all callback
  results, and supports SIMD parameters and host returns in tail calls.
- `common/wasm_memory.c` tracks successful growth and enforces QuickJS buffer limits.
- `common/wasm_shared_memory.c` rejects blocking waits for Worker agents.
- `interpreter/wasm_interp_{fast,classic}.c` checks invocation cancellation every
  4096 instructions, including during start functions.

QuickJS external-buffer hooks are documented in [its patch notes](../rquickjs/PATCH.md).
Reapply the WAMR patches when updating it and run the
Worker differential, feature audit, and cancellation tests. Unsupported features
are listed in [the runtime documentation](../docs/wasm.md).
