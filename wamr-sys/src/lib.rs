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
}
pub type Callback =
    unsafe extern "C" fn(*mut c_void, u32, *const Value, usize, *mut Value, usize) -> bool;
pub type Interrupted = unsafe extern "C" fn(*mut c_void) -> bool;
unsafe extern "C" {
    pub fn tk_wasm_engine() -> *mut c_void;
    pub fn tk_wasm_module(engine: *mut c_void, bytes: *const u8, length: usize) -> *mut Module;
    pub fn tk_wasm_delete(module: *mut Module);
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
        globals: *const Value,
        user: *mut c_void,
        callback: Callback,
        interrupted: Interrupted,
    ) -> bool;
    pub fn tk_wasm_error(module: *mut Module) -> *const c_char;
    pub fn tk_wasm_call(
        module: *mut Module,
        index: usize,
        args: *const Value,
        argc: usize,
        results: *mut Value,
        resultc: usize,
        user: *mut c_void,
        callback: Callback,
        interrupted: Interrupted,
    ) -> bool;
    pub fn tk_wasm_memory(
        module: *mut Module,
        index: usize,
        length: *mut usize,
        generation: *mut u64,
    ) -> *mut u8;
    pub fn tk_wasm_memory_grow(module: *mut Module, index: usize, delta: u32) -> i64;
    pub fn tk_wasm_global_get(module: *mut Module, index: usize) -> Value;
    pub fn tk_wasm_global_set(module: *mut Module, index: usize, value: *const Value);
}
