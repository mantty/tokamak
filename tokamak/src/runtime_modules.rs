//! The runtime's public builtin modules: their names, and their namespaces.

use rquickjs::{Ctx, Module, Object};
use tokamak_macros::host_function;

/// Each public module name with the module it resolves to.
const PUBLIC_MODULES: &[(&str, &str)] = &[
    (
        "cloudflare:workers",
        "tokamak:builtins/cloudflare-workers.ts",
    ),
    (
        "cloudflare:sockets",
        "tokamak:builtins/cloudflare-sockets.ts",
    ),
    ("cloudflare:node", "tokamak:builtins/cloudflare-node.ts"),
    ("node:assert", "tokamak:node/assert.ts"),
    ("node:assert/strict", "tokamak:node/assert-strict.ts"),
    ("node:async_hooks", "tokamak:node/async-hooks.ts"),
    ("node:buffer", "tokamak:node/buffer.ts"),
    ("node:child_process", "tokamak:node/child-process.ts"),
    ("node:cluster", "tokamak:node/cluster.ts"),
    ("node:console", "tokamak:node/console.ts"),
    ("node:constants", "tokamak:node/constants.ts"),
    ("node:crypto", "tokamak:node/crypto.ts"),
    (
        "node:diagnostics_channel",
        "tokamak:node/diagnostics-channel.ts",
    ),
    ("node:dgram", "tokamak:node/dgram.ts"),
    ("node:dns", "tokamak:node/dns.ts"),
    ("node:dns/promises", "tokamak:node/dns-promises.ts"),
    ("node:domain", "tokamak:node/domain.ts"),
    ("node:events", "tokamak:events/events.ts"),
    ("node:fs", "node:fs"),
    ("node:fs/promises", "node:fs/promises"),
    ("node:http", "tokamak:node/http.ts"),
    ("node:http2", "tokamak:node/http2.ts"),
    ("node:inspector", "tokamak:node/inspector.ts"),
    ("node:inspector/promises", "tokamak:node/inspector.ts"),
    ("node:_http_agent", "tokamak:node/internal-http-agent.ts"),
    ("node:_http_client", "tokamak:node/internal-http-client.ts"),
    ("node:_http_common", "tokamak:node/internal-http-common.ts"),
    (
        "node:_http_incoming",
        "tokamak:node/internal-http-incoming.ts",
    ),
    (
        "node:_http_outgoing",
        "tokamak:node/internal-http-outgoing.ts",
    ),
    ("node:_http_server", "tokamak:node/internal-http-server.ts"),
    ("node:https", "tokamak:node/https.ts"),
    ("node:module", "tokamak:node/module.ts"),
    ("node:net", "tokamak:node/net.ts"),
    ("node:_tls_common", "tokamak:node/internal-tls-common.ts"),
    ("node:_tls_wrap", "tokamak:node/internal-tls-wrap.ts"),
    ("node:os", "tokamak:node/os.ts"),
    ("node:path", "tokamak:node/path.ts"),
    ("node:path/posix", "tokamak:node/path-posix.ts"),
    ("node:path/win32", "tokamak:node/path-win32.ts"),
    ("node:perf_hooks", "tokamak:node/perf-hooks.ts"),
    ("node:process", "tokamak:globals/process.ts"),
    ("node:punycode", "tokamak:node/punycode.ts"),
    ("node:querystring", "tokamak:node/querystring.ts"),
    ("node:stream", "tokamak:streams/node.js"),
    ("node:stream/consumers", "tokamak:node/stream-consumers.ts"),
    ("node:stream/promises", "tokamak:node/stream-promises.ts"),
    ("node:stream/web", "tokamak:node/stream-web.ts"),
    ("node:string_decoder", "tokamak:node/string-decoder.ts"),
    ("node:sys", "tokamak:node/util.ts"),
    ("node:timers", "tokamak:node/timers.ts"),
    ("node:timers/promises", "tokamak:node/timers-promises.ts"),
    ("node:tls", "tokamak:node/tls.ts"),
    ("node:tty", "tokamak:node/tty.ts"),
    ("node:url", "tokamak:node/url.ts"),
    ("node:util", "tokamak:node/util.ts"),
    ("node:util/types", "tokamak:node/util-types.ts"),
    ("node:zlib", "tokamak:node/zlib.ts"),
    ("node:readline", "tokamak:node/readline.ts"),
    (
        "node:readline/promises",
        "tokamak:node/readline-promises.ts",
    ),
    ("node:repl", "tokamak:node/repl.ts"),
    ("node:v8", "tokamak:node/v8.ts"),
    ("node:vm", "tokamak:node/vm.ts"),
    ("node:worker_threads", "tokamak:node/worker-threads.ts"),
    (
        "node:_stream_duplex",
        "tokamak:node/internal-stream-duplex.ts",
    ),
    (
        "node:_stream_passthrough",
        "tokamak:node/internal-stream-passthrough.ts",
    ),
    (
        "node:_stream_readable",
        "tokamak:node/internal-stream-readable.ts",
    ),
    (
        "node:_stream_transform",
        "tokamak:node/internal-stream-transform.ts",
    ),
    ("node:_stream_wrap", "tokamak:node/internal-stream-wrap.ts"),
    (
        "node:_stream_writable",
        "tokamak:node/internal-stream-writable.ts",
    ),
    ("node:sqlite", "tokamak:node/sqlite.ts"),
    ("node:test", "tokamak:node/test.ts"),
    ("node:trace_events", "tokamak:node/trace-events.ts"),
    ("node:wasi", "tokamak:node/wasi.ts"),
];

/// Node modules imported only by their `node:` name.
const PREFIX_ONLY: &[&str] = &["node:test"];

/// Node modules `node:module`'s `builtinModules` leaves out.
const UNLISTED: &[&str] = &["node:sqlite", "node:test"];

/// Every public import spelling the runtime resolver handles.
#[cfg(all(test, feature = "native"))]
pub(crate) fn runtime_module_names() -> Vec<&'static str> {
    let names = PUBLIC_MODULES.iter().map(|(name, _)| *name);
    names.clone().chain(names.filter_map(bare_name)).collect()
}

pub(crate) fn public_module(name: &str) -> Option<&'static str> {
    PUBLIC_MODULES.iter().find_map(|(public, target)| {
        (*public == name || bare_name(public) == Some(name)).then_some(*target)
    })
}

/// The name a Node module is also imported by, without `node:`.
fn bare_name(public: &str) -> Option<&str> {
    public
        .strip_prefix("node:")
        .filter(|_| !PREFIX_ONLY.contains(&public))
}

/// Whether `name` names a Node module, as `node:module`'s `isBuiltin` decides.
pub(crate) fn is_node_builtin(name: &str) -> bool {
    name.starts_with("node:") || (!name.contains(':') && public_module(name).is_some())
}

/// The names `node:module`'s `builtinModules` lists, in order.
#[host_function]
pub(crate) fn node_builtin_names() -> Vec<&'static str> {
    let mut names: Vec<_> = PUBLIC_MODULES
        .iter()
        .filter(|(public, _)| !UNLISTED.contains(public))
        .filter_map(|(public, _)| public.strip_prefix("node:"))
        .collect();
    names.sort_unstable();
    names
}

/// The namespace of the builtin module `name` names, evaluated now, or
/// `None` when it names none.
pub(crate) fn builtin_namespace<'js>(
    ctx: Ctx<'js>,
    name: &str,
) -> rquickjs::Result<Option<Object<'js>>> {
    let Some(target) = public_module(name) else {
        return Ok(None);
    };
    module_namespace(ctx, target).map(Some)
}

/// The namespace of the runtime module `target`, evaluated now.
pub(crate) fn module_namespace<'js>(ctx: Ctx<'js>, target: &str) -> rquickjs::Result<Object<'js>> {
    // Evaluating a module that re-exports the namespace reaches it synchronously.
    let source = format!("export * as namespace from {target:?};");
    let module = Module::declare(ctx, format!("{target}#namespace"), source)?;
    let (module, evaluation) = module.eval()?;
    evaluation.result::<()>().transpose()?;
    module.get("namespace")
}
