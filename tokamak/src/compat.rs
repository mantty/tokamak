use rquickjs::{Ctx, Module};

pub(crate) const BOOTSTRAP: &str = "tokamak:builtins/runtime.mjs";

include!(concat!(env!("OUT_DIR"), "/builtins.rs"));

pub(crate) fn bytecode(name: &str) -> Option<&'static [u8]> {
    BUILTINS
        .iter()
        .find_map(|(path, bytecode)| (*path == name).then_some(*bytecode))
}

pub(crate) fn public_module(name: &str) -> Option<&'static str> {
    match name {
        "cloudflare:workers" => Some("tokamak:builtins/cloudflare-workers.mjs"),
        "node:events" | "events" => Some("tokamak:events/events.mjs"),
        "node:stream" | "stream" => Some("tokamak:streams/node.mjs"),
        "node:process" | "process" => Some("tokamak:globals/process.mjs"),
        "node:fs" | "fs" => Some("node:fs"),
        "node:fs/promises" | "fs/promises" => Some("node:fs/promises"),
        _ => None,
    }
}

pub(crate) fn initialize(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Module::import(ctx, BOOTSTRAP)?.finish::<()>()
}
