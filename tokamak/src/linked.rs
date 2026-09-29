//! Parts of the runtime an app links only when it uses them.
//!
//! The rest of the runtime never refers to a part directly. The app's link
//! step keeps a part by exporting its entry point, and the runtime finds the
//! entry point among its own image's exported symbols, so an app that does
//! not export it links without the part and everything only it uses.

use std::ffi::{CStr, CString, c_void};
use std::fmt::Debug;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rquickjs::{Ctx, Module};

use crate::env_vars::StorageBinding;
use crate::packaging::PackageLayout;

/// The storage part's entry point.
pub(crate) struct StorageEntry {
    /// Open the stores `bindings` name, in a storage directory and, for
    /// temporary files, a scratch directory.
    pub(crate) open: fn(&Path, &Path, &PackageLayout, &[StorageBinding]) -> OpenedStorage,
}

/// The stores the storage part opened.
pub(crate) type OpenedStorage = io::Result<Arc<dyn StorageRuntime>>;

/// The stores behind an app's storage bindings, as the runtime uses them.
pub(crate) trait StorageRuntime: Debug + Send + Sync {
    /// Give the request running in `ctx` the app's storage bindings.
    fn attach(self: Arc<Self>, ctx: &Ctx<'_>) -> rquickjs::Result<()>;

    /// The module `name`, when it is a storage module.
    fn module<'js>(&self, ctx: &Ctx<'js>, name: &str) -> Option<rquickjs::Result<Module<'js>>>;
}

/// The storage part, when the app's link kept it.
pub(crate) fn storage() -> Option<&'static StorageEntry> {
    let name = CString::new(crate::STORAGE_ENTRY_POINT).ok()?;
    let address = exported(&name)?;
    // SAFETY: the storage part exports its `StorageEntry` under this name.
    Some(unsafe { &*address.cast::<StorageEntry>() })
}

/// The address of the symbol `name` the image containing the runtime
/// exports.
#[cfg(unix)]
fn exported(name: &CStr) -> Option<*const c_void> {
    // SAFETY: `dladdr` describes the loaded image containing this function,
    // which `dlopen` with `RTLD_NOLOAD` only references while `dlsym` runs.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(exported as *const c_void, &raw mut info) == 0 {
            return None;
        }
        let mut image = libc::dlopen(info.dli_fname, libc::RTLD_NOLOAD | libc::RTLD_LAZY);
        if image.is_null() {
            // glibc does not know the main executable by its path, only as
            // the null path.
            image = libc::dlopen(std::ptr::null(), libc::RTLD_LAZY);
        }
        if image.is_null() {
            return None;
        }
        let symbol = libc::dlsym(image, name.as_ptr());
        libc::dlclose(image);
        (!symbol.is_null()).then_some(symbol.cast_const())
    }
}

/// The address of the symbol `name` the image containing the runtime
/// exports.
#[cfg(windows)]
fn exported(name: &CStr) -> Option<*const c_void> {
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
        fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
    }
    let mut image = std::ptr::null_mut();
    // SAFETY: the flags name the image containing this function by address,
    // without changing its reference count.
    unsafe {
        let address = exported as *const u16;
        if GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address, &raw mut image) == 0 {
            return None;
        }
        let symbol = GetProcAddress(image, name.as_ptr());
        (!symbol.is_null()).then_some(symbol.cast_const())
    }
}
