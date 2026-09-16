# Vendored rquickjs-sys — QuickJS engine patch

This directory is the published `rquickjs-sys` **0.12.2** crate, vendored verbatim
except for a small, deliberately minimal set of changes to the bundled QuickJS-ng
engine (`quickjs/quickjs.c` and `quickjs/quickjs.h`). Tokamak builds against this
copy through a `[patch.crates-io]` override in the workspace `Cargo.toml`.

The vendored source **is** the source of truth. This file explains every change,
why it exists in the engine (rather than in our own Rust/JS), and how to re-apply
it when the crate is upgraded.

## Why patch at all

Our runtime provides a Cloudflare-Workers-compatible JS surface. The overwhelming
majority of that surface lives in our own Rust host functions and bundled JS —
**not** here. We patch the engine only where a behaviour is impossible to obtain
from outside it because it depends on engine-internal state or an internal code
path that user-reachable JS cannot observe or intercept.

Every candidate change is first attempted in our own code. It only lands here if
that attempt fails. Two things that were tried outside the engine and **kept out**
of the patch:

- **`Array`/`TypedArray.prototype.toLocaleString` locale forwarding** — provided
  as a JS shim in `tokamak/src/builtins/globals.mjs`. Not patched.
- **ArrayBuffer clone and view reconstruction for `structuredClone`** — the
  *non-shared* ArrayBuffer clone and the whole clone algorithm live in
  `tokamak/src/globals/structured-clone.mjs`. Only the two accessors below, which
  expose engine-internal buffer state, remain in the engine.

## The changes

Total footprint: ~105 added and ~8 modified lines in `quickjs.c`, ~8 added lines
in `quickjs.h`. Grep `TOKAMAK` in the source for the one inline marker (the
class-name fix, which is otherwise invisible).

### 1. Async-context propagation across `await` (the reason this patch exists)

**Where:** `JSRuntime.async_context` and `JSPromiseReactionData.async_context`
fields; `js_enqueue_job`/`JS_EnqueueJob`; `JS_GetAsyncContext`/`JS_SetAsyncContext`;
capture at `perform_promise_then` reaction registration; save/restore around the
reaction job in `JS_ExecutePendingJob`; GC mark/free of the stored value.

**Why it cannot live outside the engine:** native `await` resumes through the
engine-internal `perform_promise_then` → `promise_reaction_job` path. That path
does **not** call the user-patchable `Promise.prototype.then`, and quickjs-ng's
`JS_SetPromiseHook` fires only around `js_promise_resolve_thenable_job`, never
around reaction jobs. There is no user-reachable or host-reachable seam on the
`await` continuation. Owning the microtask loop from Rust
(`is_job_pending`/`execute_pending_job`) does not help: a single
`execute_pending_job` call runs the reaction with no boundary we can bracket.
Proven empirically against stock quickjs before patching.

This is what makes `AsyncLocalStorage` propagate a store across `await` within a
request. `JS_GetAsyncContext` returns an owned value; `JS_SetAsyncContext` borrows
its argument and retains its own reference. Bound in
`tokamak/src/globals/async_context.rs`. No bytecode-format or public-API change.

### 2. `JS_GetClassName` returns the class-name atom, not the class-id

**Where:** one line in `JS_GetClassName` (`class_array[id].class_name`, marked
`TOKAMAK` inline).

**Why:** upstream dups the wrong struct field (`.class_id` instead of
`.class_name`), returning a meaningless atom. Class IDs are not exposed through the
`rquickjs-sys` bindings, so the "engine class of an object" — needed by
`structuredClone` to classify built-ins without tripping user getters
(`tokamak/src/globals/objects.rs`) — cannot be reconstructed in Rust. A genuine
upstream bug with no external workaround.

### 3. External-buffer `transfer`/`resize` without a crash

**Where:** `js_array_buffer_transfer` / resize path; `JS_DetachArrayBuffer` clears
the ownership callback.

**Why:** we hand many response/socket/cache bodies to JS as zero-copy
ArrayBuffers backed by a Rust `Vec` (external `free_func`, 13 production sites via
`TypedArray::new(ctx, vec)`). Stock quickjs-ng dereferences a null backing-store
pointer when such a buffer is transferred or resized (reachable from user JS via
`ArrayBuffer.prototype.transfer`/`resize`, which workerd supports). The patch
reconstructs engine-owned storage for the external case before releasing the
original. The alternative — copying every body into an engine-owned buffer at
creation — defeats the zero-copy design on the hot path.

### 4. `JS_CloneSharedArrayBuffer` — shared-growth-preserving SAB clone

**Where:** new `JS_CloneSharedArrayBuffer`; `JSArrayBuffer.ref_count` field with
init and a decrement guard in the finalizer.

**Why the obvious alternative fails:** `structuredClone` of a `SharedArrayBuffer`
must yield a clone that shares the backing store **and** shared growth (per spec
and workerd/V8: growing one grows the other's observable length). Stock
serialization (`JS_WriteObject`/`JS_ReadObject` with `JS_WRITE_OBJ_SAB`) *was*
evaluated: `JS_ReadSharedArrayBuffer` builds a **separate** `JSArrayBuffer` with
its own `byte_length`, sharing only the data pointer — so `clone.grow(n)` would
not propagate to the original. Verified against the source and the
`shared_clones_keep_views_alive_and_release_once_after_cycles` unit test, which
asserts shared growth. Sharing the `JSArrayBuffer` descriptor is the only correct
model, and that requires reference-counting the shared descriptor. Bound in
`tokamak/src/globals/buffers.rs`; the non-shared ArrayBuffer clone deliberately
uses stock serialization instead and is **not** part of this patch.

### 5. `JS_GetArrayBufferView` — view metadata without user-observable reads

**Where:** new `JS_GetArrayBufferView` returning buffer, offset, length, and the
length-tracking flag for a typed array or `DataView`.

**Why it cannot live outside the engine:** reconstructing a view for
`structuredClone` needs three pieces of engine-internal state that JS cannot read
safely: the length-tracking flag (`ta->track_rab`), out-of-bounds detection
(`typed_array_is_oob`/`dataview_is_oob`, which must raise `DataCloneError`), and
shadowing-immune access to the backing buffer (a malicious view can shadow
`.buffer`/`.byteOffset`, which is a security property). A JS re-home was attempted
and regressed all three against the workerd differential. Bound in
`tokamak/src/globals/buffers.rs`.

## Upgrading the vendored crate

1. Fetch the new pristine crate (e.g. `cargo download` or from
   `~/.cargo/registry/src/.../rquickjs-sys-<version>`) and copy it over this
   directory, preserving `PATCH.md`.
2. Re-apply the five changes above. To see the exact current hunks, diff this tree
   against the pristine 0.12.2 crate:
   ```sh
   diff -u <pristine>/quickjs/quickjs.c rquickjs/quickjs/quickjs.c
   diff -u <pristine>/quickjs/quickjs.h rquickjs/quickjs/quickjs.h
   ```
3. Before adding any *new* engine change, try to provide it from
   `tokamak/src/...` first (see `AGENTS.md`). Only patch when that is proven
   impossible, and document it here.
4. Run `cargo test -p tokamak --features native`. The workerd differential
   (`bundled_worker_matches_cloudflare_node_compat`) and the `buffers` /
   `async_context` unit tests cover every change here.

## Build wiring

`build.rs` declares `cargo:rerun-if-changed=quickjs` (plus `quickjs.bind.h` and
`src/bindings`), so the C engine is recompiled only when this directory changes,
and tokamak always links the latest build of the vendored crate.
