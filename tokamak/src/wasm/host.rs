#![allow(clippy::needless_pass_by_value)]
use super::engine::{self, Module};
use crate::dispatcher::ExecutionInterrupt;
use rquickjs::class::{Trace, Tracer};
use rquickjs::function::Args;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    ArrayBuffer, BigInt, Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, IntoJs,
    JsLifetime, Object, Value, qjs,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use tokamak_wamr_sys as ffi;

#[derive(JsLifetime)]
#[rquickjs::class]
struct Runtime<'js> {
    native: Rc<engine::Runtime>,
    imports: RefCell<HashMap<usize, Vec<Value<'js>>>>,
    buffers: RefCell<Vec<Buffer<'js>>>,
}
#[derive(JsLifetime)]
#[rquickjs::class]
struct Handle {
    native: Rc<Module>,
    instantiated: bool,
}
impl<'js> Trace<'js> for Handle {
    fn trace<'a>(&self, _: Tracer<'a, 'js>) {}
}
#[derive(JsLifetime)]
struct Buffer<'js> {
    id: u32,
    pointer: *mut u8,
    length: usize,
    generation: u64,
    value: ArrayBuffer<'js>,
}
impl<'js> Trace<'js> for Runtime<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        for imports in self.imports.borrow().values() {
            imports.trace(tracer);
        }
        for buffer in self.buffers.borrow().iter() {
            buffer.value.as_value().trace(tracer);
        }
    }
}
pub(super) struct HostModule;
crate::globals::host_functions!(
    pub(super), "runtime" => runtime, "load" => load, "info" => info,
    "instantiate" => instantiate, "exportId" => export_id, "objectInfo" => object_info,
    "call" => call, "memory" => memory, "grow" => grow,
    "globalNew" => global_new, "globalGet" => global_get, "globalSet" => global_set,
    "tableNew" => table_new, "tableSize" => table_size, "tableGet" => table_get, "tableSet" => table_set, "tableGrow" => table_grow,
);
impl ModuleDef for HostModule {
    fn declare(exports: &Declarations<'_>) -> rquickjs::Result<()> {
        for name in HOST_EXPORTS {
            exports.declare(*name)?;
        }
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        export_host_functions(ctx, exports)
    }
}
fn error(ctx: &Ctx<'_>, kind: &str, message: &str) -> rquickjs::Error {
    let make = || -> rquickjs::Result<Value<'_>> {
        let namespace: Object = ctx.globals().get("WebAssembly")?;
        let constructor: Constructor = namespace.get(kind)?;
        constructor.construct((message,))
    };
    match make() {
        Ok(value) => ctx.throw(value),
        Err(error) => error,
    }
}
fn runtime(ctx: Ctx<'_>) -> rquickjs::Result<Class<'_, Runtime<'_>>> {
    let native = engine::Runtime::new()
        .ok_or_else(|| Exception::throw_message(&ctx, "Failed to create WebAssembly store"))?;
    Class::instance(
        ctx,
        Runtime {
            native: Rc::new(native),
            imports: RefCell::default(),
            buffers: RefCell::default(),
        },
    )
}
fn load<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    bytes: ArrayBuffer<'js>,
) -> rquickjs::Result<Class<'js, Handle>> {
    let bytes = bytes
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached WebAssembly bytes"))?;
    let module = Module::new(runtime.borrow().native.clone(), bytes.to_vec()).ok_or_else(|| {
        error(
            &ctx,
            "CompileError",
            "Invalid or unsupported WebAssembly module",
        )
    })?;
    Class::instance(
        ctx,
        Handle {
            native: Rc::new(module),
            instantiated: false,
        },
    )
}
fn json<'js>(ctx: &Ctx<'js>, value: &impl serde::Serialize) -> rquickjs::Result<Value<'js>> {
    let json = serde_json::to_string(value)
        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))?;
    ctx.json_parse(json)
}
fn info<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle>,
    imports: bool,
) -> rquickjs::Result<Value<'js>> {
    let handle = handle.borrow();
    json(
        &ctx,
        if imports {
            &handle.native.imports
        } else {
            &handle.native.exports
        },
    )
}
fn instantiate<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    source: Class<'js, Handle>,
    imports: Vec<Value<'js>>,
) -> rquickjs::Result<Class<'js, Handle>> {
    let runtime = runtime.borrow();
    let module = Module::new(runtime.native.clone(), source.borrow().native.bytes.clone())
        .ok_or_else(|| error(&ctx, "CompileError", "Failed to load WebAssembly module"))?;
    if imports.len() != module.imports.len() {
        return Err(Exception::throw_type(&ctx, "Invalid import count"));
    }
    let ids = imports
        .iter()
        .map(|value| {
            if value.is_function() {
                Ok(u32::MAX)
            } else {
                u32::from_js(&ctx, value.clone())
            }
        })
        .collect::<rquickjs::Result<Vec<_>>>()?;
    runtime
        .imports
        .borrow_mut()
        .insert(module.pointer.as_ptr() as usize, imports);
    let mut frame = Frame {
        ctx: ctx.clone(),
        runtime: &runtime,
        exception: None,
    };
    // SAFETY: imports and callback frame live through synchronous instantiation.
    let ok = unsafe {
        ffi::tk_wasm_instantiate(
            module.pointer.as_ptr(),
            ids.as_ptr(),
            (&raw mut frame).cast(),
            callback,
            interrupted,
        )
    };
    refresh_buffers(&runtime);
    let kind = if unsafe { ffi::tk_wasm_link_error(module.pointer.as_ptr()) } {
        "LinkError"
    } else {
        "RuntimeError"
    };
    frame.finish(ok, kind)?;
    Class::instance(
        ctx,
        Handle {
            native: Rc::new(module),
            instantiated: true,
        },
    )
}
fn export_id(ctx: Ctx<'_>, handle: Class<'_, Handle>, index: usize) -> rquickjs::Result<u32> {
    let handle = handle.borrow();
    if !handle.instantiated || index >= handle.native.exports.len() {
        return Err(Exception::throw_type(&ctx, "Invalid Wasm export"));
    }
    // SAFETY: the index names an export of this instantiated module.
    allocation(&ctx, unsafe {
        ffi::tk_wasm_export(handle.native.pointer.as_ptr(), index)
    })
}
fn object_info<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
) -> rquickjs::Result<Value<'js>> {
    let runtime = runtime.borrow();
    let ty = runtime
        .native
        .object_type(id)
        .ok_or_else(|| Exception::throw_type(&ctx, "Invalid WebAssembly object"))?;
    json(&ctx, &ty)
}
fn require_object(
    ctx: &Ctx<'_>,
    runtime: &Runtime<'_>,
    id: u32,
    kind: &str,
) -> rquickjs::Result<engine::Type> {
    runtime
        .native
        .object_type(id)
        .filter(|ty| ty.kind == kind)
        .ok_or_else(|| Exception::throw_type(ctx, "Invalid WebAssembly object"))
}
fn call<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    arguments: Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let runtime = runtime.borrow();
    let ty = require_object(&ctx, &runtime, id, "function")?;
    let arguments = ty
        .params
        .iter()
        .zip(arguments)
        .map(|(kind, value)| to_wasm(&ctx, *kind, value))
        .collect::<rquickjs::Result<Vec<_>>>()?;
    if arguments.len() != ty.params.len() {
        return Err(Exception::throw_type(&ctx, "Invalid Wasm argument count"));
    }
    let mut results = vec![ffi::Value::default(); ty.results.len()];
    let mut frame = Frame {
        ctx: ctx.clone(),
        runtime: &runtime,
        exception: None,
    };
    // SAFETY: the signature fixes both vectors; execution and callbacks are synchronous.
    let ok = unsafe {
        ffi::tk_wasm_call(
            runtime.native.0.as_ptr(),
            id,
            arguments.as_ptr(),
            results.as_mut_ptr(),
            (&raw mut frame).cast(),
            callback,
            interrupted,
        )
    };
    refresh_buffers(&runtime);
    frame.finish(ok, "RuntimeError")?;
    match results.len() {
        0 => Ok(Value::new_undefined(ctx)),
        1 => from_wasm(&ctx, results[0]),
        _ => results
            .into_iter()
            .map(|value| from_wasm(&ctx, value))
            .collect::<rquickjs::Result<Vec<_>>>()?
            .into_js(&ctx),
    }
}
struct Frame<'a, 'js> {
    ctx: Ctx<'js>,
    runtime: &'a Runtime<'js>,
    exception: Option<Value<'js>>,
}
impl Frame<'_, '_> {
    fn finish(&mut self, ok: bool, kind: &str) -> rquickjs::Result<()> {
        if let Some(exception) = self.exception.take() {
            return Err(self.ctx.throw(exception));
        }
        if !ok {
            return Err(error(&self.ctx, kind, &self.runtime.native.error()));
        }
        Ok(())
    }
}
unsafe extern "C" fn interrupted(user: *mut c_void) -> bool {
    // SAFETY: the bridge only invokes callbacks while the frame is active.
    let frame = unsafe { &*user.cast::<Frame<'_, '_>>() };
    frame
        .ctx
        .userdata::<ExecutionInterrupt>()
        .is_some_and(|interrupt| (interrupt.0)())
}
unsafe extern "C" fn callback(
    user: *mut c_void,
    module: *mut ffi::Module,
    index: u32,
    args: *const ffi::Value,
    argument_count: usize,
    results: *mut ffi::Value,
    output_count: usize,
) -> bool {
    // SAFETY: the interpreter supplies valid vectors and a live synchronous frame.
    let frame = unsafe { &mut *user.cast::<Frame<'_, '_>>() };
    let args = unsafe { std::slice::from_raw_parts(args, argument_count) };
    let results = unsafe { std::slice::from_raw_parts_mut(results, output_count) };
    let result = invoke_import(frame, module, index as usize, args, results);
    if result.is_err() {
        frame.exception = Some(frame.ctx.catch());
    }
    result.is_ok()
}
fn invoke_import(
    frame: &Frame<'_, '_>,
    module: *mut ffi::Module,
    index: usize,
    args: &[ffi::Value],
    results: &mut [ffi::Value],
) -> rquickjs::Result<()> {
    refresh_buffers(frame.runtime);
    let value = frame.runtime.imports.borrow()[&(module as usize)][index].clone();
    let function = Function::from_js(&frame.ctx, value)?;
    let mut arguments = Args::new(frame.ctx.clone(), args.len());
    for argument in args {
        arguments.push_arg(from_wasm(&frame.ctx, *argument)?)?;
    }
    let result: Value = function.call_arg(arguments)?;
    let values = match results.len() {
        0 => Vec::new(),
        1 => vec![result],
        _ => Vec::<Value>::from_js(&frame.ctx, result)?,
    };
    if values.len() != results.len() {
        return Err(Exception::throw_type(
            &frame.ctx,
            "Wrong WebAssembly import result count",
        ));
    }
    for (i, (result, value)) in results.iter_mut().zip(values).enumerate() {
        // SAFETY: the callback signature identifies this result slot.
        let kind = unsafe { ffi::tk_wasm_value_kind(module, true, index, true, i) };
        *result = to_wasm(&frame.ctx, kind, value)?;
    }
    Ok(())
}
#[allow(
    clippy::cast_possible_truncation,
    reason = "Wasm f32 parameters round JavaScript numbers to f32"
)]
fn to_wasm<'js>(ctx: &Ctx<'js>, kind: u8, value: Value<'js>) -> rquickjs::Result<ffi::Value> {
    let data = match kind {
        0 => ffi::ValueData {
            i32: Coerced::<i32>::from_js(ctx, value)?.0,
        },
        1 => ffi::ValueData {
            i64: BigInt::from_js(ctx, value)?.to_i64()?,
        },
        2 => ffi::ValueData {
            f32: Coerced::<f64>::from_js(ctx, value)?.0 as f32,
        },
        3 => ffi::ValueData {
            f64: Coerced::<f64>::from_js(ctx, value)?.0,
        },
        128 | 129 => ffi::ValueData {
            bits: u64::from(u32::from_js(ctx, value)?),
        },
        _ => {
            return Err(Exception::throw_type(
                ctx,
                "Reference and SIMD values cannot cross this WebAssembly boundary",
            ));
        }
    };
    Ok(ffi::Value {
        kind,
        data,
        ..ffi::Value::default()
    })
}
fn from_wasm<'js>(ctx: &Ctx<'js>, value: ffi::Value) -> rquickjs::Result<Value<'js>> {
    match value.kind {
        // SAFETY: WAMR's tag identifies the initialized member of its union.
        0 => unsafe { value.data.i32 }.into_js(ctx),
        1 => BigInt::from_i64(ctx.clone(), unsafe { value.data.i64 })?.into_js(ctx),
        2 => unsafe { value.data.f32 }.into_js(ctx),
        3 => unsafe { value.data.f64 }.into_js(ctx),
        128 | 129 => f64::from(unsafe { value.data.i32 }.cast_unsigned()).into_js(ctx),
        _ => Err(Exception::throw_type(
            ctx,
            "Unsupported WebAssembly result type",
        )),
    }
}

fn refresh_buffers(runtime: &Runtime<'_>) {
    for buffer in runtime.buffers.borrow_mut().iter_mut() {
        let (mut length, mut generation) = (0, 0);
        // SAFETY: the buffer retains this store and a valid memory id.
        let pointer = unsafe {
            ffi::tk_wasm_memory(
                runtime.native.0.as_ptr(),
                buffer.id,
                &raw mut length,
                &raw mut generation,
            )
        };
        if pointer != buffer.pointer || length != buffer.length || generation != buffer.generation {
            buffer.value.detach();
        }
    }
}
unsafe extern "C" fn release_buffer(_: *mut qjs::JSRuntime, opaque: *mut c_void, _: *mut c_void) {
    // SAFETY: each external buffer consumes exactly one retained store owner.
    drop(unsafe { Rc::from_raw(opaque.cast::<engine::Runtime>()) });
}
fn memory<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "memory")?;
    refresh_buffers(&runtime);
    let (mut length, mut generation) = (0, 0);
    // SAFETY: ids originate in the private JS wrapper's memory state.
    let pointer = unsafe {
        ffi::tk_wasm_memory(
            runtime.native.0.as_ptr(),
            id,
            &raw mut length,
            &raw mut generation,
        )
    };
    let mut buffers = runtime.buffers.borrow_mut();
    if let Some(buffer) = buffers
        .iter()
        .find(|buffer| buffer.id == id && buffer.generation == generation)
    {
        return Ok(buffer.value.clone());
    }
    buffers.retain(|buffer| buffer.id != id);
    let value = if length == 0 {
        ArrayBuffer::new(ctx.clone(), Vec::<u8>::new())?
    } else {
        // SAFETY: the store owns this allocation; growth detaches all JS aliases before JS resumes.
        unsafe {
            let owner = Rc::into_raw(runtime.native.clone());
            let raw = qjs::JS_NewArrayBuffer(
                ctx.as_raw().as_ptr(),
                pointer,
                length as _,
                Some(release_buffer),
                owner.cast_mut().cast(),
                false,
            );
            let value = Value::from_raw(ctx.clone(), raw);
            if value.is_exception() {
                drop(Rc::from_raw(owner));
                return Err(rquickjs::Error::Exception);
            }
            ArrayBuffer::from_js(&ctx, value)?
        }
    };
    buffers.push(Buffer {
        id,
        pointer,
        length,
        generation,
        value: value.clone(),
    });
    Ok(value)
}
fn grow<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    delta: u32,
) -> rquickjs::Result<i64> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "memory")?;
    // SAFETY: the wrapper supplies a memory id; WAMR enforces its bounds.
    let previous = unsafe { ffi::tk_wasm_memory_grow(runtime.native.0.as_ptr(), id, delta) };
    refresh_buffers(&runtime);
    if previous < 0 {
        return Err(Exception::throw_range(
            &ctx,
            "WebAssembly memory maximum exceeded",
        ));
    }
    Ok(previous)
}
fn allocation(ctx: &Ctx<'_>, id: u32) -> rquickjs::Result<u32> {
    if id == u32::MAX {
        Err(Exception::throw_range(ctx, "WebAssembly allocation failed"))
    } else {
        Ok(id)
    }
}
fn global_new<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    kind: u8,
    mutable: bool,
    value: Value<'js>,
) -> rquickjs::Result<u32> {
    let value = to_wasm(&ctx, kind, value)?;
    // SAFETY: the value's tag and storage agree.
    allocation(&ctx, unsafe {
        ffi::tk_wasm_new_global(
            runtime.borrow().native.0.as_ptr(),
            kind,
            mutable,
            &raw const value,
        )
    })
}
fn global_get<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
) -> rquickjs::Result<Value<'js>> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "global")?;
    // SAFETY: the registry confirms this id identifies a global.
    from_wasm(&ctx, unsafe {
        ffi::tk_wasm_global_get(runtime.native.0.as_ptr(), id)
    })
}
fn global_set<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let runtime = runtime.borrow();
    let ty = require_object(&ctx, &runtime, id, "global")?;
    if !ty.mutable {
        return Err(Exception::throw_type(
            &ctx,
            "WebAssembly global is immutable",
        ));
    }
    let value = to_wasm(&ctx, ty.value, value)?;
    // SAFETY: the global is mutable and the value is converted to its type.
    unsafe {
        ffi::tk_wasm_global_set(runtime.native.0.as_ptr(), id, &raw const value);
    }
    Ok(())
}
fn table_new<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    kind: u8,
    initial: u32,
    maximum: u32,
    value: u32,
) -> rquickjs::Result<u32> {
    if !matches!(kind, 128 | 129) || maximum < initial {
        return Err(Exception::throw_type(
            &ctx,
            "Invalid WebAssembly table descriptor",
        ));
    }
    // SAFETY: the descriptor has a reference type and valid limits.
    allocation(&ctx, unsafe {
        ffi::tk_wasm_new_table(
            runtime.borrow().native.0.as_ptr(),
            kind,
            initial,
            maximum,
            value,
        )
    })
}
fn table_size<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
) -> rquickjs::Result<f64> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "table")?;
    // SAFETY: the registry confirms this id identifies a table.
    Ok(f64::from(unsafe {
        ffi::tk_wasm_table_size(runtime.native.0.as_ptr(), id)
    }))
}
fn table_get<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    index: u32,
) -> rquickjs::Result<f64> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "table")?;
    // SAFETY: bounds are checked before indexing the table.
    unsafe {
        if index >= ffi::tk_wasm_table_size(runtime.native.0.as_ptr(), id) {
            return Err(Exception::throw_range(
                &ctx,
                "WebAssembly table index out of bounds",
            ));
        }
        Ok(f64::from(ffi::tk_wasm_table_get(
            runtime.native.0.as_ptr(),
            id,
            index,
        )))
    }
}
fn table_set<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    index: u32,
    value: u32,
) -> rquickjs::Result<()> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "table")?;
    // SAFETY: the JS wrapper converts the reference; bounds precede the native write.
    unsafe {
        if index >= ffi::tk_wasm_table_size(runtime.native.0.as_ptr(), id) {
            return Err(Exception::throw_range(
                &ctx,
                "WebAssembly table index out of bounds",
            ));
        }
        ffi::tk_wasm_table_set(runtime.native.0.as_ptr(), id, index, value);
    }
    Ok(())
}
fn table_grow<'js>(
    ctx: Ctx<'js>,
    runtime: Class<'js, Runtime<'js>>,
    id: u32,
    delta: u32,
    value: u32,
) -> rquickjs::Result<i64> {
    let runtime = runtime.borrow();
    require_object(&ctx, &runtime, id, "table")?;
    // SAFETY: the id identifies a table; WAMR validates growth against its limits.
    let previous = unsafe { ffi::tk_wasm_table_grow(runtime.native.0.as_ptr(), id, delta, value) };
    if previous < 0 {
        Err(Exception::throw_range(
            &ctx,
            "WebAssembly table maximum exceeded",
        ))
    } else {
        Ok(previous)
    }
}
