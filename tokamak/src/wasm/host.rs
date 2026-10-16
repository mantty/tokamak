#![allow(clippy::needless_pass_by_value)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use rquickjs::class::{Trace, Tracer};
use rquickjs::function::Args;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    ArrayBuffer, BigInt, Class, Coerced, Constructor, Ctx, Exception, FromJs, Function, IntoJs,
    JsLifetime, Object, Value, qjs,
};
use tokamak_wamr_sys as ffi;

use super::engine::Module;
use crate::dispatcher::ExecutionInterrupt;

#[derive(JsLifetime)]
#[rquickjs::class]
struct Handle<'js> {
    native: Rc<Module>,
    imports: Vec<Value<'js>>,
    buffers: RefCell<Vec<Buffer<'js>>>,
}
#[derive(JsLifetime)]
struct Buffer<'js> {
    index: usize,
    pointer: *mut u8,
    length: usize,
    generation: u64,
    value: ArrayBuffer<'js>,
}
impl<'js> Trace<'js> for Handle<'js> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.imports.trace(tracer);
        for buffer in self.buffers.borrow().iter() {
            buffer.value.as_value().trace(tracer);
        }
    }
}

pub(super) struct HostModule;
crate::globals::host_functions!(
    pub(super),
    "load" => load,
    "info" => info,
    "instantiate" => instantiate,
    "call" => call,
    "memory" => memory,
    "grow" => grow,
    "globalGet" => global_get,
    "globalSet" => global_set,
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

fn load<'js>(ctx: Ctx<'js>, bytes: ArrayBuffer<'js>) -> rquickjs::Result<Class<'js, Handle<'js>>> {
    let bytes = bytes
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached WebAssembly bytes"))?;
    let module = Module::new(bytes.to_vec()).ok_or_else(|| {
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
            imports: Vec::new(),
            buffers: RefCell::default(),
        },
    )
}

fn info<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    imports: bool,
) -> rquickjs::Result<Value<'js>> {
    let handle = handle.borrow();
    let types = if imports {
        &handle.native.imports
    } else {
        &handle.native.exports
    };
    let json = serde_json::to_string(types)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    ctx.json_parse(json)
}

fn instantiate<'js>(
    ctx: Ctx<'js>,
    source: Class<'js, Handle<'js>>,
    imports: Vec<Value<'js>>,
) -> rquickjs::Result<Class<'js, Handle<'js>>> {
    let module = Module::new(source.borrow().native.bytes.clone())
        .ok_or_else(|| error(&ctx, "CompileError", "Failed to load WebAssembly module"))?;
    if imports.len() != module.imports.len() {
        return Err(Exception::throw_type(&ctx, "Invalid import count"));
    }
    let mut globals = vec![ffi::Value::default(); imports.len()];
    for (index, ty) in module.imports.iter().enumerate() {
        match ty.kind {
            "function" if imports[index].is_function() => {}
            "global" if !ty.mutable => {
                globals[index] = to_wasm(&ctx, ty.value, imports[index].clone())?;
            }
            "global" => {
                return Err(error(
                    &ctx,
                    "LinkError",
                    "WAMR does not support imported mutable globals",
                ));
            }
            "function" => {
                return Err(error(
                    &ctx,
                    "LinkError",
                    "WebAssembly function import is not callable",
                ));
            }
            _ => {
                return Err(error(
                    &ctx,
                    "LinkError",
                    "WAMR does not support imported memories or tables",
                ));
            }
        }
    }
    let handle = Class::instance(
        ctx.clone(),
        Handle {
            native: Rc::new(module),
            imports,
            buffers: RefCell::default(),
        },
    )?;
    let borrowed = handle.borrow();
    let mut frame = Frame {
        ctx: ctx.clone(),
        handle: &borrowed,
        exception: None,
    };
    // SAFETY: the frame and import values live through this synchronous call.
    let ok = unsafe {
        ffi::tk_wasm_instantiate(
            borrowed.native.pointer.as_ptr(),
            globals.as_ptr(),
            (&raw mut frame).cast(),
            callback,
            interrupted,
        )
    };
    frame.finish(ok, "RuntimeError")?;
    drop(borrowed);
    Ok(handle)
}

fn call<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    index: usize,
    arguments: Vec<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let handle = handle.borrow();
    let ty = handle
        .native
        .exports
        .get(index)
        .filter(|ty| ty.kind == "function")
        .ok_or_else(|| Exception::throw_type(&ctx, "Invalid WebAssembly function"))?;
    let arguments = ty
        .params
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            to_wasm(
                &ctx,
                *kind,
                arguments
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| Value::new_undefined(ctx.clone())),
            )
        })
        .collect::<rquickjs::Result<Vec<_>>>()?;
    let mut results = vec![ffi::Value::default(); ty.results.len()];
    let mut frame = Frame {
        ctx: ctx.clone(),
        handle: &handle,
        exception: None,
    };
    // SAFETY: metadata fixes vector lengths and the callback cannot outlive frame.
    let ok = unsafe {
        ffi::tk_wasm_call(
            handle.native.pointer.as_ptr(),
            index,
            arguments.as_ptr(),
            arguments.len(),
            results.as_mut_ptr(),
            results.len(),
            (&raw mut frame).cast(),
            callback,
            interrupted,
        )
    };
    refresh_buffers(&handle);
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
    handle: &'a Handle<'js>,
    exception: Option<Value<'js>>,
}
impl Frame<'_, '_> {
    fn finish(&mut self, ok: bool, kind: &str) -> rquickjs::Result<()> {
        if let Some(exception) = self.exception.take() {
            return Err(self.ctx.throw(exception));
        }
        if !ok {
            return Err(error(&self.ctx, kind, &self.handle.native.error()));
        }
        Ok(())
    }
}
unsafe extern "C" fn interrupted(user: *mut c_void) -> bool {
    // SAFETY: the bridge only calls this while the frame is active.
    let frame = unsafe { &*user.cast::<Frame<'_, '_>>() };
    frame
        .ctx
        .userdata::<ExecutionInterrupt>()
        .is_some_and(|interrupt| (interrupt.0)())
}
unsafe extern "C" fn callback(
    user: *mut c_void,
    index: u32,
    args: *const ffi::Value,
    argument_count: usize,
    results: *mut ffi::Value,
    output_count: usize,
) -> bool {
    // SAFETY: WAMR supplies valid vectors for the declared signature, and the
    // bridge calls back synchronously while this frame and its JS values live.
    let frame = unsafe { &mut *user.cast::<Frame<'_, '_>>() };
    let args = if argument_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args, argument_count) }
    };
    let results = if output_count == 0 {
        &mut []
    } else {
        unsafe { std::slice::from_raw_parts_mut(results, output_count) }
    };
    let result = invoke_import(frame, index as usize, args, results);
    if result.is_err() {
        frame.exception = Some(frame.ctx.catch());
    }
    result.is_ok()
}
fn invoke_import(
    frame: &Frame<'_, '_>,
    index: usize,
    args: &[ffi::Value],
    results: &mut [ffi::Value],
) -> rquickjs::Result<()> {
    refresh_buffers(frame.handle);
    let function = Function::from_js(&frame.ctx, frame.handle.imports[index].clone())?;
    let mut arguments = Args::new(frame.ctx.clone(), args.len());
    for argument in args {
        arguments.push_arg(from_wasm(&frame.ctx, *argument)?)?;
    }
    let result: Value = function.call_arg(arguments)?;
    let kinds = &frame.handle.native.imports[index].results;
    let values = match kinds.len() {
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
    for ((result, kind), value) in results.iter_mut().zip(kinds).zip(values) {
        *result = to_wasm(&frame.ctx, *kind, value)?;
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
        _ => Err(Exception::throw_type(
            ctx,
            "Unsupported WebAssembly result type",
        )),
    }
}

fn refresh_buffers(handle: &Handle<'_>) {
    for buffer in handle.buffers.borrow_mut().iter_mut() {
        let mut length = 0;
        let mut generation = 0;
        // SAFETY: buffer.index names a memory export of this live instance.
        let pointer = unsafe {
            ffi::tk_wasm_memory(
                handle.native.pointer.as_ptr(),
                buffer.index,
                &raw mut length,
                &raw mut generation,
            )
        };
        if pointer != buffer.pointer || length != buffer.length || generation != buffer.generation {
            buffer.value.detach();
        }
    }
}
unsafe extern "C" fn release_buffer(
    _runtime: *mut qjs::JSRuntime,
    opaque: *mut c_void,
    _pointer: *mut c_void,
) {
    // SAFETY: each buffer consumes exactly one Rc passed to JS_NewArrayBuffer.
    drop(unsafe { Rc::from_raw(opaque.cast::<Module>()) });
}
fn memory<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    index: usize,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let handle = handle.borrow();
    if handle
        .native
        .exports
        .get(index)
        .is_none_or(|ty| ty.kind != "memory")
    {
        return Err(Exception::throw_type(&ctx, "Invalid memory export"));
    }
    refresh_buffers(&handle);
    let mut buffers = handle.buffers.borrow_mut();
    let mut length = 0;
    let mut generation = 0;
    // SAFETY: index names a memory export of this live instance.
    let pointer = unsafe {
        ffi::tk_wasm_memory(
            handle.native.pointer.as_ptr(),
            index,
            &raw mut length,
            &raw mut generation,
        )
    };
    if let Some(buffer) = buffers.iter().find(|buffer| {
        buffer.index == index
            && buffer.generation == generation
            && buffer.pointer == pointer
            && buffer.length == length
    }) {
        return Ok(buffer.value.clone());
    }
    buffers.retain(|buffer| buffer.index != index);
    // SAFETY: the memory export stays alive through the buffer's Rc. Views are
    // detached after growth before JavaScript resumes, including host callbacks.
    let value = if length == 0 {
        ArrayBuffer::new(ctx.clone(), Vec::<u8>::new())?
    } else {
        unsafe {
            let owner = Rc::into_raw(handle.native.clone());
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
        index,
        pointer,
        length,
        generation,
        value: value.clone(),
    });
    Ok(value)
}
fn grow<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    index: usize,
    delta: u32,
) -> rquickjs::Result<i64> {
    let handle = handle.borrow();
    if handle
        .native
        .exports
        .get(index)
        .is_none_or(|ty| ty.kind != "memory")
    {
        return Err(Exception::throw_type(&ctx, "Invalid memory export"));
    }
    // SAFETY: index names a memory of this instance; WAMR checks its maximum.
    let previous =
        unsafe { ffi::tk_wasm_memory_grow(handle.native.pointer.as_ptr(), index, delta) };
    if previous < 0 {
        return Err(Exception::throw_range(
            &ctx,
            "WebAssembly memory maximum exceeded",
        ));
    }
    refresh_buffers(&handle);
    Ok(previous)
}
fn global_get<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    index: usize,
) -> rquickjs::Result<Value<'js>> {
    let handle = handle.borrow();
    if handle
        .native
        .exports
        .get(index)
        .is_none_or(|ty| ty.kind != "global")
    {
        return Err(Exception::throw_type(&ctx, "Invalid global export"));
    }
    // SAFETY: index names a global in this live instance.
    from_wasm(&ctx, unsafe {
        ffi::tk_wasm_global_get(handle.native.pointer.as_ptr(), index)
    })
}
fn global_set<'js>(
    ctx: Ctx<'js>,
    handle: Class<'js, Handle<'js>>,
    index: usize,
    value: Value<'js>,
) -> rquickjs::Result<()> {
    let handle = handle.borrow();
    let ty = handle
        .native
        .exports
        .get(index)
        .filter(|ty| ty.kind == "global" && ty.mutable)
        .ok_or_else(|| Exception::throw_type(&ctx, "WebAssembly global is immutable"))?;
    let value = to_wasm(&ctx, ty.value, value)?;
    // SAFETY: index names a mutable global of the converted type.
    unsafe {
        ffi::tk_wasm_global_set(handle.native.pointer.as_ptr(), index, &raw const value);
    }
    Ok(())
}
