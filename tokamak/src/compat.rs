use rquickjs::{Ctx, Module};

pub(crate) const BOOTSTRAP: &str = "tokamak:builtins/runtime.mjs";

include!(concat!(env!("OUT_DIR"), "/builtins.rs"));

pub(crate) fn bytecode(name: &str) -> Option<&'static [u8]> {
    BUILTINS
        .iter()
        .find_map(|(path, bytecode)| (*path == name).then_some(*bytecode))
}

pub(crate) fn initialize(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Module::import(ctx, BOOTSTRAP)?.finish::<()>()
}
