use std::alloc::{Layout, alloc, dealloc};
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering, fence};

use rquickjs::module::Exports;
use rquickjs::{Ctx, Object, Value, qjs};

use super::checked;

pub(super) fn export<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
    install(ctx);
    exports.export(
        "cloneArrayBuffer",
        rquickjs::Function::new(ctx.clone(), clone)?,
    )?;
    exports.export(
        "arrayBufferView",
        rquickjs::Function::new(ctx.clone(), view)?,
    )?;
    Ok(())
}

unsafe extern "C" {
    fn JS_CloneSharedArrayBuffer(ctx: *mut qjs::JSContext, source: qjs::JSValue) -> qjs::JSValue;
    fn JS_GetArrayBufferView(
        ctx: *mut qjs::JSContext,
        source: qjs::JSValue,
        offset: *mut usize,
        length: *mut usize,
        tracking: *mut bool,
    ) -> qjs::JSValue;
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn view<'js>(ctx: Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Object<'js>> {
    let (mut offset, mut length, mut tracking) = (0, 0, false);
    // SAFETY: The engine validates the view and returns an owned buffer value.
    // Metadata outputs point to live variables with C size_t/bool layouts.
    let buffer = checked(unsafe {
        Value::from_raw(
            ctx.clone(),
            JS_GetArrayBufferView(
                ctx.as_raw().as_ptr(),
                value.as_raw(),
                &raw mut offset,
                &raw mut length,
                &raw mut tracking,
            ),
        )
    })?;
    let info = Object::new(ctx)?;
    info.set("buffer", buffer)?;
    info.set("offset", offset)?;
    info.set("length", length)?;
    info.set("tracking", tracking)?;
    Ok(info)
}

#[repr(C, align(8))]
struct SharedAllocation {
    references: AtomicUsize,
    layout: Layout,
}

#[allow(clippy::cast_ptr_alignment)] // The allocator uses SharedAllocation's alignment.
unsafe extern "C" fn allocate(_opaque: *mut c_void, size: qjs::size_t) -> *mut c_void {
    let Ok(size) = usize::try_from(size) else {
        return ptr::null_mut();
    };
    let Some(size) = size.checked_add(size_of::<SharedAllocation>()) else {
        return ptr::null_mut();
    };
    let Ok(layout) = Layout::from_size_align(size, align_of::<SharedAllocation>()) else {
        return ptr::null_mut();
    };
    // SAFETY: The nonzero layout reserves the aligned header and requested bytes.
    unsafe {
        let allocation = alloc(layout).cast::<SharedAllocation>();
        if allocation.is_null() {
            return ptr::null_mut();
        }
        allocation.write(SharedAllocation {
            references: AtomicUsize::new(1),
            layout,
        });
        allocation.add(1).cast()
    }
}

unsafe extern "C" fn retain(_opaque: *mut c_void, data: *mut c_void) {
    // SAFETY: QuickJS passes a live pointer returned by allocate, after its header.
    unsafe {
        (*data.cast::<SharedAllocation>().sub(1))
            .references
            .fetch_add(1, Ordering::Relaxed);
    }
}

unsafe extern "C" fn release(_opaque: *mut c_void, data: *mut c_void) {
    // SAFETY: Every retained owner releases once. The last owner frees the same
    // layout used for allocation, after acquiring prior owners' writes.
    unsafe {
        let allocation = data.cast::<SharedAllocation>().sub(1);
        if (*allocation).references.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);
            let layout = (*allocation).layout;
            ptr::drop_in_place(allocation);
            dealloc(allocation.cast(), layout);
        }
    }
}

pub(super) fn install(ctx: &Ctx<'_>) {
    let functions = qjs::JSSharedArrayBufferFunctions {
        sab_alloc: Some(allocate),
        sab_free: Some(release),
        sab_dup: Some(retain),
        sab_opaque: ptr::null_mut(),
    };
    // SAFETY: Installation precedes user code and any SharedArrayBuffer creation.
    // QuickJS copies the callback table; the functions have static lifetime.
    unsafe {
        qjs::JS_SetSharedArrayBufferFunctions(
            qjs::JS_GetRuntime(ctx.as_raw().as_ptr()),
            &raw const functions,
        );
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn clone<'js>(
    ctx: Ctx<'js>,
    value: Value<'js>,
    shared: bool,
) -> rquickjs::Result<Value<'js>> {
    if shared {
        // SAFETY: Source and destination belong to the same runtime. The engine
        // validates the class and retains the shared backing store descriptor.
        return checked(unsafe {
            Value::from_raw(
                ctx.clone(),
                JS_CloneSharedArrayBuffer(ctx.as_raw().as_ptr(), value.as_raw()),
            )
        });
    }
    // SAFETY: No script executes in the codec. Only a live buffer is serialized,
    // and bytecode reading is disabled. The serialized bytes are always freed.
    let copied = unsafe {
        let raw_ctx = ctx.as_raw().as_ptr();
        let mut size = 0;
        if qjs::JS_GetArrayBuffer(raw_ctx, &raw mut size, value.as_raw()).is_null() {
            return Err(rquickjs::Error::Exception);
        }
        let bytes = qjs::JS_WriteObject(raw_ctx, &raw mut size, value.as_raw(), 0);
        if bytes.is_null() {
            return Err(rquickjs::Error::Exception);
        }
        let copied = qjs::JS_ReadObject(raw_ctx, bytes, size, 0);
        qjs::js_free(raw_ctx, bytes.cast());
        Value::from_raw(ctx, copied)
    };
    checked(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{Context, Function, Runtime};

    struct Allocations {
        allocated: AtomicUsize,
        released: AtomicUsize,
    }

    unsafe extern "C" fn counted_allocate(opaque: *mut c_void, size: qjs::size_t) -> *mut c_void {
        // SAFETY: The test keeps this state alive until the runtime is dropped.
        unsafe {
            let data = allocate(opaque, size);
            if !data.is_null() {
                (*opaque.cast::<Allocations>())
                    .allocated
                    .fetch_add(1, Ordering::Relaxed);
            }
            data
        }
    }

    unsafe extern "C" fn counted_release(opaque: *mut c_void, data: *mut c_void) {
        // SAFETY: These are the matching callback state and allocation pointers.
        unsafe {
            (*opaque.cast::<Allocations>())
                .released
                .fetch_add(1, Ordering::Relaxed);
            release(opaque, data);
        }
    }

    #[test]
    fn shared_clones_keep_views_alive_and_release_once_after_cycles() -> rquickjs::Result<()> {
        let counts = Allocations {
            allocated: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
        };
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> rquickjs::Result<()> {
            let functions = qjs::JSSharedArrayBufferFunctions {
                sab_alloc: Some(counted_allocate),
                sab_free: Some(counted_release),
                sab_dup: Some(retain),
                sab_opaque: (&raw const counts).cast_mut().cast(),
            };
            // SAFETY: This copies the table; counts outlives context and runtime.
            unsafe {
                qjs::JS_SetSharedArrayBufferFunctions(
                    qjs::JS_GetRuntime(ctx.as_raw().as_ptr()),
                    &raw const functions,
                );
            }
            ctx.globals()
                .set("cloneBuffer", Function::new(ctx.clone(), clone)?)?;
            ctx.eval::<(), _>(
                r"
                globalThis.source = new SharedArrayBuffer(4, { maxByteLength: 16 });
                globalThis.copy = cloneBuffer(source, true);
                globalThis.originalView = new Uint8Array(source);
                globalThis.copyView = new DataView(copy);
                source.cycle = source;
                copy.cycle = copyView;
                copyView.cycle = copy;
                source = null;
            ",
            )?;
            Ok(())
        })?;
        runtime.run_gc();
        assert_eq!(counts.released.load(Ordering::Relaxed), 0);
        context.with(|ctx| -> rquickjs::Result<()> {
            assert!(ctx.eval::<bool, _>(
                r"
                copy.grow(8);
                originalView[0] = 29;
                copyView.setUint8(1, 41);
                originalView.length === 8 && copyView.byteLength === 8 &&
                    copyView.getUint8(0) === 29 && originalView[1] === 41
            "
            )?);
            ctx.eval::<(), _>("copy = null; copyView = null; originalView = null;")?;
            Ok(())
        })?;
        runtime.run_gc();
        assert_eq!(counts.allocated.load(Ordering::Relaxed), 1);
        assert_eq!(counts.released.load(Ordering::Relaxed), 1);
        // GC may visit the wrappers/views in either order. Repeat cyclic graphs
        // with both fixed and growable stores before destroying the context.
        context.with(|ctx| {
            ctx.eval::<(), _>(
                r"
            for (let i = 0; i < 100; i++) {
                const a = new SharedArrayBuffer(8, i % 2 ? { maxByteLength: 16 } : undefined);
                const b = cloneBuffer(a, true), c = cloneBuffer(b, true);
                a.other = b; b.other = c; c.other = a;
                c.view = new Uint8Array(a);
            }
        ",
            )
        })?;
        drop(context);
        runtime.run_gc();
        drop(runtime);
        assert_eq!(counts.allocated.load(Ordering::Relaxed), 101);
        assert_eq!(counts.released.load(Ordering::Relaxed), 101);
        Ok(())
    }
}
