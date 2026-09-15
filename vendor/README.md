# QuickJS integration

`rquickjs-sys-0.12.2` contains the published crate and its bundled QuickJS source.
The local extension retains an opaque JavaScript context value across jobs and
promise reactions, including native `await` continuations. Reaction context is
captured when the continuation is registered, not when its promise settles.
The engine traces and releases these references with their owning jobs/reactions.

`JS_GetAsyncContext` returns an owned value. `JS_SetAsyncContext` borrows its
argument and retains its own reference. These APIs are private host integration;
they do not alter the bytecode format or expose a new application API.

Upstream licenses remain in the vendored crate. Runtime differential and
garbage-collection tests cover the extension.

External ArrayBuffer detachment clears its ownership callback. Same-size
transfers move both the buffer and callback state; resized transfers copy into
engine-owned storage before releasing the original allocation.

`JS_GetClassName` returns the registered name atom, independently of class IDs.

`JS_CloneSharedArrayBuffer` retains a shared backing-store descriptor within one
runtime. Clones observe the same bytes and current length after growth; the last
wrapper releases the descriptor and allocation.

`JS_GetArrayBufferView` reads view metadata without invoking user properties,
including the length-tracking flag needed to clone resizable-buffer views.

Array and TypedArray `toLocaleString` forward the locale and options arguments
to each element's method, allowing the runtime's ICU-backed methods to apply.
