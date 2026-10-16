//! Ownership and metadata for the C interpreter.
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;
use std::sync::OnceLock;

use serde::Serialize;
use tokamak_wamr_sys as ffi;

pub(super) struct Module {
    pub(super) pointer: NonNull<ffi::Module>,
    pub(super) bytes: Vec<u8>,
    pub(super) imports: Vec<Type>,
    pub(super) exports: Vec<Type>,
}

#[derive(Serialize)]
pub(super) struct Type {
    pub(super) name: String,
    pub(super) module: String,
    pub(super) kind: &'static str,
    pub(super) params: Vec<u8>,
    pub(super) results: Vec<u8>,
    pub(super) value: u8,
    pub(super) mutable: bool,
}

impl Module {
    pub(super) fn new(bytes: Vec<u8>) -> Option<Self> {
        static ENGINE: OnceLock<usize> = OnceLock::new();
        // SAFETY: WAMR's engine is initialized once and lives for the process.
        // Stores and their objects remain on their creating QuickJS thread.
        let engine = *ENGINE.get_or_init(|| unsafe { ffi::tk_wasm_engine() as usize });
        if engine == 0 {
            return None;
        }
        // SAFETY: WAMR copies and validates the bytes before returning.
        let pointer = NonNull::new(unsafe {
            ffi::tk_wasm_module(engine as *mut c_void, bytes.as_ptr(), bytes.len())
        })?;
        let mut module = Self {
            pointer,
            bytes,
            imports: Vec::new(),
            exports: Vec::new(),
        };
        module.imports = module.types(true);
        module.exports = module.types(false);
        Some(module)
    }

    fn types(&self, imports: bool) -> Vec<Type> {
        let pointer = self.pointer.as_ptr();
        // SAFETY: all indices come from WAMR's corresponding metadata vector.
        unsafe {
            (0..ffi::tk_wasm_count(pointer, imports))
                .map(|index| {
                    let info = ffi::tk_wasm_type(pointer, imports, index);
                    let kinds = |results, count| {
                        (0..count)
                            .map(|argument| {
                                ffi::tk_wasm_value_kind(
                                    pointer,
                                    imports,
                                    index,
                                    results,
                                    argument as usize,
                                )
                            })
                            .collect()
                    };
                    Type {
                        name: name(info.name, info.name_len),
                        module: name(info.module, info.module_len),
                        kind: ["function", "global", "table", "memory"][info.kind as usize],
                        params: kinds(false, info.params),
                        results: kinds(true, info.results),
                        value: info.value_kind,
                        mutable: info.mutable_value != 0,
                    }
                })
                .collect()
        }
    }

    pub(super) fn error(&self) -> String {
        // SAFETY: the bridge owns a terminated error string while self lives.
        unsafe { CStr::from_ptr(ffi::tk_wasm_error(self.pointer.as_ptr())) }
            .to_string_lossy()
            .into_owned()
    }
}

unsafe fn name(pointer: *const std::ffi::c_char, length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    // SAFETY: the caller supplies WAMR's live name and its byte length.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast(), length) };
    String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or(bytes)).into_owned()
}

impl Drop for Module {
    fn drop(&mut self) {
        // SAFETY: this is the unique owning handle; external buffers retain it.
        unsafe {
            ffi::tk_wasm_delete(self.pointer.as_ptr());
        }
    }
}
