//! Opaque bindings to the pinned WAMR interpreter and Tokamak's C bridge.
use std::ffi::{c_char, c_void};
use std::mem::MaybeUninit;

#[repr(C)]
pub struct Module {
    _private: [u8; 0],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Value {
    pub kind: u8,
    pub padding: [MaybeUninit<u8>; 7],
    pub data: ValueData,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub union ValueData {
    pub i32: i32,
    pub i64: i64,
    pub f32: f32,
    pub f64: f64,
    pub bits: u64,
}
impl Default for Value {
    fn default() -> Self {
        Self {
            kind: 0,
            padding: [MaybeUninit::new(0); 7],
            data: ValueData { bits: 0 },
        }
    }
}
#[repr(C)]
pub struct Type {
    pub name: *const c_char,
    pub name_len: usize,
    pub module: *const c_char,
    pub module_len: usize,
    pub kind: u32,
    pub params: u32,
    pub results: u32,
    pub value_kind: u8,
    pub mutable_value: u32,
    pub function_index: u32,
}
#[repr(C)]
pub struct Runtime {
    _private: [u8; 0],
}
pub type Callback = unsafe extern "C" fn(
    *mut c_void,
    *mut Module,
    u32,
    *const Value,
    usize,
    *mut Value,
    usize,
) -> bool;
pub type Interrupted = unsafe extern "C" fn(*mut c_void) -> bool;
unsafe extern "C" {
    pub fn tk_wasm_engine() -> *mut c_void;
    pub fn tk_wasm_runtime(engine: *mut c_void) -> *mut Runtime;
    pub fn tk_wasm_runtime_delete(runtime: *mut Runtime);
    pub fn tk_wasm_module(runtime: *mut Runtime, bytes: *const u8, length: usize) -> *mut Module;
    pub fn tk_wasm_count(module: *mut Module, imports: bool) -> usize;
    pub fn tk_wasm_type(module: *mut Module, imports: bool, index: usize) -> Type;
    pub fn tk_wasm_value_kind(
        module: *mut Module,
        imports: bool,
        index: usize,
        results: bool,
        argument: usize,
    ) -> u8;
    pub fn tk_wasm_instantiate(
        module: *mut Module,
        imports: *const u32,
        user: *mut c_void,
        callback: Callback,
        interrupted: Interrupted,
    ) -> bool;
    pub fn tk_wasm_link_error(module: *mut Module) -> bool;
    pub fn tk_wasm_error(runtime: *mut Runtime) -> *const c_char;
    pub fn tk_wasm_export(module: *mut Module, index: usize) -> u32;
    pub fn tk_wasm_object_type(runtime: *mut Runtime, id: u32) -> Type;
    pub fn tk_wasm_object_value_kind(
        runtime: *mut Runtime,
        id: u32,
        results: bool,
        argument: usize,
    ) -> u8;
    pub fn tk_wasm_call(
        runtime: *mut Runtime,
        id: u32,
        args: *const Value,
        results: *mut Value,
        user: *mut c_void,
        callback: Callback,
        interrupted: Interrupted,
    ) -> bool;
    pub fn tk_wasm_memory(
        runtime: *mut Runtime,
        id: u32,
        length: *mut usize,
        generation: *mut u64,
    ) -> *mut u8;
    pub fn tk_wasm_memory_maximum(runtime: *mut Runtime, id: u32) -> u64;
    pub fn tk_wasm_memory_shared(runtime: *mut Runtime, id: u32) -> bool;
    pub fn tk_wasm_memory_grow(runtime: *mut Runtime, id: u32, delta: u32) -> i64;
    pub fn tk_wasm_new_global(
        runtime: *mut Runtime,
        kind: u8,
        mutable: bool,
        value: *const Value,
    ) -> u32;
    pub fn tk_wasm_global_get(runtime: *mut Runtime, id: u32) -> Value;
    pub fn tk_wasm_global_set(runtime: *mut Runtime, id: u32, value: *const Value);
    pub fn tk_wasm_new_table(
        runtime: *mut Runtime,
        kind: u8,
        initial: u32,
        maximum: u32,
        value: u32,
    ) -> u32;
    pub fn tk_wasm_table_size(runtime: *mut Runtime, id: u32) -> u32;
    pub fn tk_wasm_table_get(runtime: *mut Runtime, id: u32, index: u32) -> u32;
    pub fn tk_wasm_table_set(runtime: *mut Runtime, id: u32, index: u32, value: u32);
    pub fn tk_wasm_table_grow(runtime: *mut Runtime, id: u32, delta: u32, value: u32) -> i64;
}
