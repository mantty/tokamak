//! Optional WebAssembly runtime, reached only through its linked entry point.
mod engine;
mod host;

use crate::linked::WasmEntry;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{ArrayBuffer, Ctx, Function, Module, Object};

include!(concat!(env!("OUT_DIR"), "/wasm_builtins.rs"));

#[unsafe(export_name = "tokamak_wasm")]
static ENTRY: WasmEntry = WasmEntry {
    install,
    module,
    compiled,
};

fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Module::import(ctx, "tokamak:wasm/api.mjs")?.finish::<Object>()?;
    Ok(())
}

fn module<'js>(ctx: &Ctx<'js>, name: &str) -> Option<rquickjs::Result<Module<'js>>> {
    if name == "tokamak:wasm/host" {
        return Some(Module::declare_def::<host::HostModule, _>(
            ctx.clone(),
            name,
        ));
    }
    crate::compat::find_bytecode(WASM_BUILTINS, name).map(|bytes| {
        // SAFETY: these modules are compiled with this runtime's QuickJS.
        unsafe { Module::load(ctx.clone(), bytes) }
    })
}

fn compiled<'js>(ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
    Module::declare_def::<CompiledModule, _>(ctx.clone(), name)
}

struct CompiledModule;
impl ModuleDef for CompiledModule {
    fn declare(exports: &Declarations<'_>) -> rquickjs::Result<()> {
        exports.declare("default")?;
        Ok(())
    }
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let bytes = crate::dispatcher::module_file(ctx, exports)?;
        let api: Object = Module::import(ctx, "tokamak:wasm/api.mjs")?.finish()?;
        let create: Function = api.get("compiledModule")?;
        let module: Object = create.call((ArrayBuffer::new(ctx.clone(), bytes)?,))?;
        exports.export("default", module)?;
        Ok(())
    }
}
