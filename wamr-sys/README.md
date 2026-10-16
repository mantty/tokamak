# WAMR embedding

`vendor/` contains `core/`, `build-scripts/`, and `LICENSE` from
[WAMR 2.4.5](https://github.com/wasm-micro-runtime/wasm-micro-runtime/releases/tag/WAMR-2.4.5).
Cargo builds the vendored C sources without downloading build-time dependencies.

The interpreter uses software bounds checks and does not allocate executable
memory. JIT, AOT, WASI, native libc imports, threads, SIMD, and dynamic module
linking are disabled. Each JavaScript instance owns its WAMR store. The process
owns one WAMR engine. Stores stay on their creating runtime thread.

`bridge.c` confines access to WAMR's internal types. The Rust interface uses
opaque handles, explicit vector lengths, and the C API's tagged numeric union.
Memory views retain the instance's store; growth detaches old JavaScript buffers
before JavaScript resumes, including callbacks from Wasm.

## Upstream changes

- `build-scripts/version.cmake` uses the pinned `core/version.h` without rewriting
  the source tree.
- `interpreter/wasm.h` adds `c_api_instantiation` to loaded modules.
- `common/wasm_c_api.c` marks C API modules and executes their start function
  after installing imports. A failed start returns its trap.
- `interpreter/wasm_runtime.c` skips automatic native startup hooks for C API
  modules. JavaScript instantiation runs the Wasm start section only; exported
  `__wasm_call_ctors`, `_initialize`, and `__post_instantiate` are ordinary exports.
- `interpreter/wasm_loader.c` allows memory growth from the JavaScript host even
  when the module contains no `memory.grow` instruction.
- `common/wasm_memory.c` and `interpreter/wasm_runtime.h` track successful memory
  growth, including zero-page growth, to detach all exported buffer aliases.
- `interpreter/wasm_interp_{fast,classic}.c` polls the active invocation's
  cancellation callback every 4096 instructions. The callback also covers start
  functions. Execution remains synchronous on the JavaScript thread.

These changes do not modify QuickJS. Reapply them when updating WAMR and run the
Worker differential and cancellation tests.
