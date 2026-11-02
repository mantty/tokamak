use rquickjs::{Ctx, Module, Object};

use crate::dispatcher::Bootstrap;

pub(crate) const BOOTSTRAP: &str = "tokamak:builtins/runtime.ts";

include!(concat!(env!("OUT_DIR"), "/builtins.rs"));

pub(crate) fn bytecode(name: &str) -> Option<&'static [u8]> {
    find_bytecode(BUILTINS, name)
}

/// The bytecode of the builtin module `name` in `table`.
pub(crate) fn find_bytecode(table: &[(&str, &'static [u8])], name: &str) -> Option<&'static [u8]> {
    table
        .iter()
        .find_map(|(path, bytecode)| (*path == name).then_some(*bytecode))
}

/// Evaluate the runtime bootstrap, returning its functions: the dispatcher's
/// routes into the runtime.
pub(crate) fn initialize<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Bootstrap<'js>> {
    let exports: Object = Module::import(ctx, BOOTSTRAP)?.finish()?;
    exports.get("bootstrap")
}
