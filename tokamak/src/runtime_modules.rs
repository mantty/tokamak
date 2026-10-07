//! The public names of the runtime's builtin modules.

use rquickjs::{Ctx, Module, Object};

/// Each public module name with the module it resolves to.
const PUBLIC_MODULES: &[(&str, &str)] = &[
    (
        "cloudflare:workers",
        "tokamak:builtins/cloudflare-workers.mjs",
    ),
    (
        "cloudflare:sockets",
        "tokamak:builtins/cloudflare-sockets.mjs",
    ),
    ("cloudflare:node", "tokamak:builtins/cloudflare-node.mjs"),
    ("node:assert", "tokamak:node/assert.mjs"),
    ("node:assert/strict", "tokamak:node/assert-strict.mjs"),
    ("node:async_hooks", "tokamak:node/async-hooks.mjs"),
    ("node:buffer", "tokamak:node/buffer.mjs"),
    ("node:child_process", "tokamak:node/child-process.mjs"),
    ("node:cluster", "tokamak:node/cluster.mjs"),
    ("node:console", "tokamak:node/console.mjs"),
    ("node:constants", "tokamak:node/constants.mjs"),
    ("node:crypto", "tokamak:node/crypto.mjs"),
    (
        "node:diagnostics_channel",
        "tokamak:node/diagnostics-channel.mjs",
    ),
    ("node:dgram", "tokamak:node/dgram.mjs"),
    ("node:dns", "tokamak:node/dns.mjs"),
    ("node:dns/promises", "tokamak:node/dns-promises.mjs"),
    ("node:domain", "tokamak:node/domain.mjs"),
    ("node:events", "tokamak:events/events.mjs"),
    ("node:fs", "node:fs"),
    ("node:fs/promises", "node:fs/promises"),
    ("node:http", "tokamak:node/http.mjs"),
    ("node:http2", "tokamak:node/http2.mjs"),
    ("node:inspector", "tokamak:node/inspector.mjs"),
    ("node:inspector/promises", "tokamak:node/inspector.mjs"),
    ("node:_http_agent", "tokamak:node/internal-http-agent.mjs"),
    ("node:_http_client", "tokamak:node/internal-http-client.mjs"),
    ("node:_http_common", "tokamak:node/internal-http-common.mjs"),
    (
        "node:_http_incoming",
        "tokamak:node/internal-http-incoming.mjs",
    ),
    (
        "node:_http_outgoing",
        "tokamak:node/internal-http-outgoing.mjs",
    ),
    ("node:_http_server", "tokamak:node/internal-http-server.mjs"),
    ("node:https", "tokamak:node/https.mjs"),
    ("node:module", "tokamak:node/module.mjs"),
    ("node:net", "tokamak:node/net.mjs"),
    ("node:_tls_common", "tokamak:node/internal-tls-common.mjs"),
    ("node:_tls_wrap", "tokamak:node/internal-tls-wrap.mjs"),
    ("node:os", "tokamak:node/os.mjs"),
    ("node:path", "tokamak:node/path.mjs"),
    ("node:path/posix", "tokamak:node/path-posix.mjs"),
    ("node:path/win32", "tokamak:node/path-win32.mjs"),
    ("node:perf_hooks", "tokamak:node/perf-hooks.mjs"),
    ("node:process", "tokamak:globals/process.mjs"),
    ("node:punycode", "tokamak:node/punycode.mjs"),
    ("node:querystring", "tokamak:node/querystring.mjs"),
    ("node:stream", "tokamak:streams/node.mjs"),
    ("node:stream/consumers", "tokamak:node/stream-consumers.mjs"),
    ("node:stream/promises", "tokamak:node/stream-promises.mjs"),
    ("node:stream/web", "tokamak:node/stream-web.mjs"),
    ("node:string_decoder", "tokamak:node/string-decoder.mjs"),
    ("node:sys", "tokamak:node/util.mjs"),
    ("node:timers", "tokamak:node/timers.mjs"),
    ("node:timers/promises", "tokamak:node/timers-promises.mjs"),
    ("node:tls", "tokamak:node/tls.mjs"),
    ("node:tty", "tokamak:node/tty.mjs"),
    ("node:url", "tokamak:node/url.mjs"),
    ("node:util", "tokamak:node/util.mjs"),
    ("node:util/types", "tokamak:node/util-types.mjs"),
    ("node:zlib", "tokamak:node/zlib.mjs"),
    ("node:readline", "tokamak:node/readline.mjs"),
    (
        "node:readline/promises",
        "tokamak:node/readline-promises.mjs",
    ),
    ("node:repl", "tokamak:node/repl.mjs"),
    ("node:v8", "tokamak:node/v8.mjs"),
    ("node:vm", "tokamak:node/vm.mjs"),
    ("node:worker_threads", "tokamak:node/worker-threads.mjs"),
    (
        "node:_stream_duplex",
        "tokamak:node/internal-stream-duplex.mjs",
    ),
    (
        "node:_stream_passthrough",
        "tokamak:node/internal-stream-passthrough.mjs",
    ),
    (
        "node:_stream_readable",
        "tokamak:node/internal-stream-readable.mjs",
    ),
    (
        "node:_stream_transform",
        "tokamak:node/internal-stream-transform.mjs",
    ),
    ("node:_stream_wrap", "tokamak:node/internal-stream-wrap.mjs"),
    (
        "node:_stream_writable",
        "tokamak:node/internal-stream-writable.mjs",
    ),
    ("node:sqlite", "tokamak:node/sqlite.mjs"),
    ("node:test", "tokamak:node/test.mjs"),
    ("node:trace_events", "tokamak:node/trace-events.mjs"),
    ("node:wasi", "tokamak:node/wasi.mjs"),
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
    // Evaluating a module that re-exports the namespace reaches it synchronously.
    let source = format!("export * as namespace from {target:?};");
    let module = Module::declare(ctx, format!("{target}#namespace"), source)?;
    let (module, evaluation) = module.eval()?;
    evaluation.result::<()>().transpose()?;
    module.get("namespace").map(Some)
}
