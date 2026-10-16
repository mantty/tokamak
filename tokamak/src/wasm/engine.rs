//! Native store ownership and WebAssembly type metadata.
use serde::Serialize;
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::OnceLock;
use tokamak_wamr_sys as ffi;

pub(super) struct Runtime(pub(super) NonNull<ffi::Runtime>);
impl Runtime {
    pub(super) fn new() -> Option<Self> {
        static ENGINE: OnceLock<usize> = OnceLock::new();
        // SAFETY: one process engine; each store stays on its creating JS thread.
        let engine = *ENGINE.get_or_init(|| unsafe { ffi::tk_wasm_engine() as usize });
        if engine == 0 {
            return None;
        }
        NonNull::new(unsafe { ffi::tk_wasm_runtime(engine as *mut c_void) }).map(Self)
    }
    pub(super) fn error(&self) -> String {
        // SAFETY: the store owns a terminated error string.
        unsafe { CStr::from_ptr(ffi::tk_wasm_error(self.0.as_ptr())) }
            .to_string_lossy()
            .into_owned()
    }
    pub(super) fn object_type(&self, id: u32) -> Option<Type> {
        // SAFETY: the registry checks the id before returning type metadata.
        unsafe {
            let info = ffi::tk_wasm_object_type(self.0.as_ptr(), id);
            (info.kind < 4).then(|| {
                Type::read(&info, |results, index| {
                    ffi::tk_wasm_object_value_kind(self.0.as_ptr(), id, results, index)
                })
            })
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // SAFETY: the last owner releases the store after external buffers.
        unsafe {
            ffi::tk_wasm_runtime_delete(self.0.as_ptr());
        }
    }
}
pub(super) struct Module {
    pub(super) pointer: NonNull<ffi::Module>,
    _runtime: Rc<Runtime>,
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
    pub(super) function_index: u32,
}
impl Type {
    fn read(info: &ffi::Type, kind: impl Fn(bool, usize) -> u8) -> Self {
        Self {
            name: name(info.name, info.name_len),
            module: name(info.module, info.module_len),
            kind: ["function", "global", "table", "memory"][info.kind as usize],
            params: (0..info.params).map(|i| kind(false, i as usize)).collect(),
            results: (0..info.results).map(|i| kind(true, i as usize)).collect(),
            value: info.value_kind,
            mutable: info.mutable_value != 0,
            function_index: info.function_index,
        }
    }
}
impl Module {
    pub(super) fn new(runtime: Rc<Runtime>, bytes: Vec<u8>) -> Option<Self> {
        // SAFETY: the store copies and validates the bytes before returning.
        let pointer = NonNull::new(unsafe {
            ffi::tk_wasm_module(runtime.0.as_ptr(), bytes.as_ptr(), bytes.len())
        })?;
        let mut module = Self {
            pointer,
            _runtime: runtime,
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
        // SAFETY: indices and counts come from the module's metadata.
        unsafe {
            (0..ffi::tk_wasm_count(pointer, imports))
                .map(|index| {
                    let info = ffi::tk_wasm_type(pointer, imports, index);
                    Type::read(&info, |results, argument| {
                        ffi::tk_wasm_value_kind(pointer, imports, index, results, argument)
                    })
                })
                .collect()
        }
    }
}
fn name(pointer: *const std::ffi::c_char, length: usize) -> String {
    if length == 0 {
        return String::new();
    }
    // SAFETY: names reference the module's live metadata.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast(), length) };
    String::from_utf8_lossy(bytes.strip_suffix(&[0]).unwrap_or(bytes)).into_owned()
}
