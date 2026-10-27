use crate::certificates::Certificates;
use crate::dispatcher::{
    Dispatcher, WorkerLoader, WorkerResolver, configure_worker_loader, execute_request, load_worker,
};
use crate::fs::VirtualFileSystem;
use crate::gateway::{
    Handler, Job, JobResponse, WebSocketInbound, WebSocketOutbound, websocket_channels,
};
use crate::packaging::{PackageLayout, WorkerManifest, write_worker};
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::transport::{HttpBody, HttpRequest, queue_websocket_message};
use flume::{Receiver, Sender};
use hyper::header::HeaderMap;
use rquickjs::{Context, Function, Module, Object, Runtime as JsRuntime, TypedArray};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) const HOST: &str = "example.test";

const WEBSOCKET_WORKER: &[u8] = br#"
export default {
  async fetch() {
    const [client, server] = Object.values(new WebSocketPair());
    server.accept();
    server.addEventListener("message", (event) => server.send(`pong ${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#;

const CACHE_WORKER: &[u8] = br#"
export default {
  async fetch(request) {
    const cache = await caches.open("pages");
    const key = "https://example.test/page";
    if (request.method === "PUT") {
      await cache.put(key, new Response("cached", { headers: { "x-cache": "hit" } }));
      return new Response("stored");
    }
    const entry = await cache.match(key);
    return new Response(entry ? `${await entry.text()} ${entry.headers.get("x-cache")}` : "miss");
  },
};
"#;

const GLOBAL_WORKER: &[u8] = br#"
const encoded = new TextEncoder().encode("ready");
const decoded = new TextDecoder().decode(encoded);
export default {
  async fetch() {
    return new Response(null, { status: decoded === "ready" ? 204 : 500 });
  },
};
"#;

const STREAM_WORKER: &[u8] = br#"
export default {
  fetch: async request => {
    if (new Uint8Array(await request.arrayBuffer()).join() !== "9,8,7") {
      throw new Error("request body was not transferred as bytes");
    }
    const chunks = [new Uint8Array([1, 2]), new Uint8Array([3, 4])];
    const body = new ReadableStream({
      pull(controller) {
        const chunk = chunks.shift();
        if (chunk) controller.enqueue(chunk);
        else controller.close();
      },
    });
    return new Response(body, { headers: { "content-type": "application/octet-stream" } });
  },
};
"#;

/// Registers an invocation-owned listener and responds.
const LISTENING_WORKER: &[u8] = br#"const nativeListen = (ctx, plugin, method, value, next, error) => ctx[Symbol.for("tokamak.native")].listen(plugin, method, new TextEncoder().encode(JSON.stringify(value)), { next: payload => next(JSON.parse(new TextDecoder().decode(payload))), error });

export default {
  fetch(request, env, ctx) {
    const stop = nativeListen(ctx, "location", "watchPosition", null, () => stop(), () => {});
    return new Response("listening");
  },
};
"#;

/// Answers `job` with `handler` on a blocking Tokio thread, as the gateway does.
fn handle_job(
    tokio: &tokio::runtime::Runtime,
    handler: Arc<dyn Handler>,
    job: Job,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio.spawn_blocking(move || {
        handler
            .handle(job, &CancellationToken::new())
            .map_err(|error| error.to_string())
    })
}

/// The body of `dispatcher`'s buffered response to `method /page`.
fn respond(
    tokio: &tokio::runtime::Runtime,
    dispatcher: &Arc<Dispatcher>,
    method: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (response, responses) = flume::bounded(1);
    let job = Job {
        cancelled: CancellationToken::new(),
        deadline: None,
        request: request(method, "/page"),
        response,
        websocket: None,
    };
    tokio.block_on(handle_job(tokio, dispatcher.clone(), job))??;
    let JobResponse::Http(response) = responses.recv()? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    let HttpBody::Buffered(body) = response.body else {
        return Err("Worker response was streamed".into());
    };
    Ok(String::from_utf8(body)?)
}

#[test]
fn keeps_cache_entries_for_the_life_of_their_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(CACHE_WORKER, directory.path())?;
    let tokio = tokio::runtime::Runtime::new()?;
    let running = Dispatcher::new(worker.clone(), runtime_config());
    let restarted = Dispatcher::new(worker, runtime_config());

    assert_eq!(respond(&tokio, &running, "PUT")?, "stored");
    assert_eq!(respond(&tokio, &running, "GET")?, "cached hit");
    assert_eq!(respond(&tokio, &restarted, "GET")?, "miss");
    Ok(())
}

#[test]
fn releases_invocation_owned_listeners_when_the_request_finishes()
-> Result<(), Box<dyn std::error::Error>> {
    /// Shell plugins that report each subscription's ID.
    struct Shell(Sender<u64>, Sender<u64>);
    impl crate::PluginHandler for Shell {
        fn call(&self, _: u64, _: &str, _: &str, _: &[u8]) {}

        fn call_with_body(&self, _: u64, _: &str, _: &str, _: &[u8]) {}

        fn chunk(&self, _: u64, _: &crate::PluginReply) {}

        fn subscribe(&self, id: u64, _: &str, _: &str, _: &[u8]) {
            let _ = self.0.send(id);
        }

        fn cancel(&self, id: u64) {
            let _ = self.1.send(id);
        }
    }
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(LISTENING_WORKER, directory.path())?;
    let (subscriptions, subscribed) = flume::unbounded();
    let (removed, cancelled) = flume::unbounded();
    let plugins =
        crate::plugin_calls::PluginCalls::new(Some(Arc::new(Shell(subscriptions, removed))), true);
    let config = RuntimeConfig {
        plugins: Some(Arc::clone(&plugins)),
        ..runtime_config()
    };
    let tokio = tokio::runtime::Runtime::new()?;
    let (response, responses) = flume::bounded(1);
    let job = Job {
        cancelled: CancellationToken::new(),
        deadline: None,
        request: request("GET", "/"),
        response,
        websocket: None,
    };

    let handled = handle_job(&tokio, Dispatcher::new(worker, config), job);
    let JobResponse::Http(response) = responses.recv_timeout(Duration::from_secs(5))? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    let subscription = subscribed.recv_timeout(Duration::from_secs(5))?;

    let HttpBody::Buffered(body) = response.body else {
        panic!("Worker response was streamed");
    };
    assert_eq!(body, b"listening");
    assert_eq!(
        cancelled.recv_timeout(Duration::from_secs(5))?,
        subscription
    );
    tokio.block_on(handled)??;
    Ok(())
}

pub(crate) fn request(method: &str, path: &str) -> HttpRequest {
    HttpRequest {
        persistent: true,
        method: method.to_owned(),
        target: path.to_owned(),
        url: format!("https://example.test{path}"),
        headers: HeaderMap::new(),
        body: None,
    }
}

#[test]
fn http_wait_until_survives_caller_cancellation_but_ends_at_the_host_deadline()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        br"const nativeCall = async (ctx, plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await ctx[Symbol.for('tokamak.native')].call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));

export default {
  async fetch(request, env, ctx) {
    ctx.waitUntil((async () => {
      await new Promise(resolve => setTimeout(resolve, 100));
      await nativeCall(ctx, 'gate', 'retained');
    })());
    await nativeCall(ctx, 'gate', 'handler');
    return new Response('finished');
  }
};",
        directory.path(),
    )?;
    let (events, received) = flume::unbounded();
    let config = RuntimeConfig {
        plugins: Some(crate::plugin_calls::PluginCalls::new(
            Some(Arc::new(Gate(events))),
            true,
        )),
        ..runtime_config()
    };
    let tokio = tokio::runtime::Runtime::new()?;
    let stopped = CancellationToken::new();
    let _shutdown = stopped.clone().drop_guard();
    let caller = CancellationToken::new();
    let (response, responses) = flume::bounded(1);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    let job = Job {
        cancelled: caller.clone(),
        deadline: Some(deadline),
        request: request("GET", "/"),
        response,
        websocket: None,
    };
    let execution = tokio.spawn_blocking(move || execute_request(&worker, &config, job, &stopped));
    assert_eq!(received.recv_timeout(Duration::from_secs(2))?.1, "handler");
    caller.cancel();
    assert_eq!(received.recv_timeout(Duration::from_secs(2))?.1, "retained");
    let finished =
        tokio.block_on(async { tokio::time::timeout(Duration::from_secs(2), execution).await });
    assert!(
        finished.is_ok(),
        "HTTP waitUntil exceeded its host deadline"
    );
    assert!(finished??.is_err(), "the host deadline was not reported");
    assert!(std::time::Instant::now() >= deadline);
    assert!(responses.try_recv().is_err());
    Ok(())
}

#[test]
fn cancels_the_http_body_while_independent_background_work_is_retained()
-> Result<(), Box<dyn std::error::Error>> {
    check_http_body_cleanup(true)
}

#[test]
fn completes_response_cancellation_cleanup_without_other_background_work()
-> Result<(), Box<dyn std::error::Error>> {
    check_http_body_cleanup(false)
}

/// Shell plugins that record each call's ID and method.
pub(crate) struct Gate(pub(crate) Sender<(u64, String)>);

impl crate::PluginHandler for Gate {
    fn call(&self, id: u64, _: &str, method: &str, _: &[u8]) {
        let _ = self.0.send((id, method.into()));
    }
    fn call_with_body(&self, _: u64, _: &str, _: &str, _: &[u8]) {}
    fn chunk(&self, _: u64, _: &crate::PluginReply) {}
    fn subscribe(&self, _: u64, _: &str, _: &str, _: &[u8]) {}
    fn cancel(&self, _: u64) {}
}

fn check_http_body_cleanup(retained: bool) -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        br"const nativeCall = async (ctx, plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await ctx[Symbol.for('tokamak.native')].call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));

const gate = (ctx, name) => nativeCall(ctx, 'gate', name);
export default {
  fetch(request, env, ctx) {
    if (env.RETAIN) ctx.waitUntil(gate(ctx, 'retained').then(() => gate(ctx, 'followup')));
    return new Response(new ReadableStream({
      pull() { return gate(ctx, 'pull'); },
      cancel() {
        let sum = 0;
        for (let i = 0; i < 50000; i++) sum += i;
        if (sum !== 1249975000) throw new Error('cleanup calculation failed');
        return gate(ctx, 'cancelled');
      },
    }));
  }
};",
        directory.path(),
    )?;
    let (sent, received) = flume::unbounded();
    let plugins = crate::plugin_calls::PluginCalls::new(Some(Arc::new(Gate(sent))), true);
    let config = RuntimeConfig {
        plugins: Some(Arc::clone(&plugins)),
        environment: [("RETAIN".into(), retained.into())].into(),
        ..runtime_config()
    };
    let tokio = tokio::runtime::Runtime::new()?;
    let stopped = CancellationToken::new();
    let _shutdown = stopped.clone().drop_guard();
    let caller = CancellationToken::new();
    let (response, responses) = flume::bounded(1);
    let job = Job {
        cancelled: caller.clone(),
        deadline: None,
        request: request("GET", "/"),
        response,
        websocket: None,
    };
    let execution = tokio.spawn_blocking(move || execute_request(&worker, &config, job, &stopped));
    let JobResponse::Http(response) = responses.recv_timeout(Duration::from_secs(5))? else {
        return Err("expected HTTP response".into());
    };
    let mut pending = std::collections::BTreeMap::new();
    for _ in 0..=usize::from(retained) {
        let (id, method) = received.recv_timeout(Duration::from_secs(5))?;
        pending.insert(method, id);
    }
    caller.cancel();
    let followup = if retained {
        plugins.reply(pending["retained"], native_completion());
        let (id, method) = received.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(method, "followup");
        Some(id)
    } else {
        None
    };
    drop(response);
    let (cleanup, method) = received.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(method, "cancelled");
    assert!(
        !execution.is_finished(),
        "background work ended with the response body"
    );
    if let Some(followup) = followup {
        plugins.reply(followup, native_completion());
    }
    plugins.reply(cleanup, native_completion());
    tokio
        .block_on(async { tokio::time::timeout(Duration::from_secs(5), execution).await })???;
    Ok(())
}

#[test]
fn routes_worker_websocket_messages_through_the_native_bridge()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker_bundle = WorkerBundle::of_source(WEBSOCKET_WORKER, directory.path())?;
    let dispatcher = Dispatcher::new(worker_bundle, runtime_config());
    let (response_sender, response_receiver) = flume::bounded(1);
    let (websocket, bridge) = websocket_channels()?;
    let incoming_sender = bridge.incoming;
    let outgoing_receiver = bridge.outgoing;
    let tokio = tokio::runtime::Runtime::new()?;
    let worker = handle_job(
        &tokio,
        dispatcher,
        Job {
            cancelled: CancellationToken::new(),
            deadline: None,
            request: request("GET", "/socket"),
            response: response_sender,
            websocket: Some(websocket),
        },
    );

    let response = response_receiver.recv_timeout(Duration::from_secs(1));
    if let Err(error) = response {
        let worker_error = tokio.block_on(worker)?;
        return Err(
            format!("WebSocket worker stopped before upgrade: {error}; {worker_error:?}").into(),
        );
    }
    let JobResponse::WebSocket = response? else {
        panic!("Worker returned a non-WebSocket response");
    };
    let WebSocketOutbound::Ready = outgoing_receiver.recv_timeout(Duration::from_secs(1))? else {
        panic!("Worker did not finish handling the upgrade");
    };
    incoming_sender.send(WebSocketInbound::Message {
        binary: false,
        payload: b"ping 42".to_vec(),
    })?;
    match outgoing_receiver.recv_timeout(Duration::from_secs(1))? {
        WebSocketOutbound::Message {
            binary: false,
            payload,
        } => assert_eq!(payload, b"pong ping 42"),
        WebSocketOutbound::Message { binary: true, .. } => panic!("Worker answered in binary"),
        WebSocketOutbound::Close { .. } => panic!("Worker closed the WebSocket"),
        WebSocketOutbound::Ready => panic!("Worker completed without a response"),
    }
    let WebSocketOutbound::Ready = outgoing_receiver.recv_timeout(Duration::from_secs(1))? else {
        panic!("Worker did not finish handling the message");
    };
    incoming_sender.send(WebSocketInbound::Close {
        code: 1000,
        reason: String::new(),
    })?;
    tokio.block_on(worker)??;
    Ok(())
}

#[test]
fn streams_worker_response_chunks_without_buffering_the_body()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker_bundle = WorkerBundle::of_source(STREAM_WORKER, directory.path())?;
    let dispatcher = Dispatcher::new(worker_bundle, runtime_config());
    let mut request = request("POST", "/upload");
    request.body = Some(vec![9, 8, 7]);
    let (response_sender, response_receiver) = flume::bounded(1);
    let tokio = tokio::runtime::Runtime::new()?;
    let worker = handle_job(
        &tokio,
        dispatcher,
        Job {
            cancelled: CancellationToken::new(),
            deadline: None,
            request,
            response: response_sender,
            websocket: None,
        },
    );

    let response = response_receiver.recv_timeout(Duration::from_secs(1))?;
    let JobResponse::Http(response) = response else {
        return Err("Worker returned a non-HTTP response".into());
    };
    let HttpBody::Stream(body) = response.body else {
        return Err("Worker response was not streamed".into());
    };
    assert_eq!(body.recv()?.map_err(io::Error::other)?, vec![1, 2]);
    assert_eq!(body.recv()?.map_err(io::Error::other)?, vec![3, 4]);
    assert!(body.recv().is_err());
    tokio.block_on(worker)??;
    Ok(())
}

#[test]
fn loads_split_worker_modules_through_the_quickjs_loader() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("chunks"))?;
    std::fs::write(
        directory.path().join("entry.js"),
        br#"import worker from "./chunks/worker.js"; export default worker;"#,
    )?;
    std::fs::write(
        directory.path().join("chunks/worker.js"),
        br"export default { fetch() {} };",
    )?;
    let manifest = WorkerManifest::es_modules("entry.js", &["chunks/worker.js"]);
    let app = PackageLayout::new(directory.path().join("app"));
    write_worker(&app, directory.path(), &manifest)?;
    let worker = WorkerBundle::new(manifest, app);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    executor.block_on(async {
        let runtime = rquickjs::AsyncRuntime::new()?;
        runtime
            .set_loader(
                WorkerResolver,
                WorkerLoader {
                    bundle: worker.clone(),
                    storage: None,
                },
            )
            .await;
        let context = rquickjs::AsyncContext::full(&runtime).await?;
        context
            .async_with(async |ctx| {
                let _: Object = load_worker(&ctx, &worker).await?;
                Ok::<_, Error>(())
            })
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}

#[test]
fn loads_builtins_without_source_compilation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = JsRuntime::new()?;
    configure_worker_loader(
        &runtime,
        &WorkerBundle::new(
            WorkerManifest::es_modules("entry.js", &[]),
            PackageLayout::new(directory.path()),
        ),
    );
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> rquickjs::Result<()> {
        let environment = Object::new(ctx.clone())?;
        environment.set("FLAG", "request environment")?;
        ctx.globals().set("__tokamak_env", environment)?;
        let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(crate::fs::Bundle::new(
            directory.path(),
        ))));
        crate::fs::install(&ctx, &vfs)?;
        crate::compat::initialize(&ctx)?;
        let module: Object = Module::import(&ctx, "cloudflare:workers")?.finish()?;
        let env: Object = module.get("env")?;
        assert_eq!(env.get::<_, String>("FLAG")?, "request environment");
        let streams: Object = Module::import(&ctx, "node:stream")?.finish()?;
        let writable: rquickjs::Constructor = streams.get("Writable")?;
        let events: Object = Module::import(&ctx, "node:events")?.finish()?;
        let emitter: Object = events.get("EventEmitter")?;
        let instance: Object = writable.construct(())?;
        assert!(instance.is_instance_of(&emitter));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn initializes_web_globals_before_worker_module_evaluation()
-> Result<(), Box<dyn std::error::Error>> {
    let JobResponse::Http(response) = run_worker(GLOBAL_WORKER, request("GET", "/"))? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    assert_eq!(response.status, 204);
    Ok(())
}

#[test]
fn names_the_platform_before_worker_module_evaluation() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"const platform = globalThis.__tokamak_platform;
export default { fetch: () => new Response(platform) };";
    let JobResponse::Http(response) = run_worker(source, request("GET", "/"))? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    assert!(
        matches!(&response.body, HttpBody::Buffered(body) if body == crate::globals::plugins::PLATFORM.as_bytes())
    );
    Ok(())
}

#[test]
fn rejects_a_handler_result_that_is_not_a_response() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"export default { fetch: () => ({ status: 200, headers: [] }) };";
    let error = run_worker(source, request("GET", "/"))
        .err()
        .ok_or("a plain object was sent as the response")?;
    assert!(
        error
            .to_string()
            .contains("Incorrect type for Promise: the Promise did not resolve to 'Response'."),
        "{error}"
    );
    Ok(())
}

/// What the Worker compiled from `source` sends for `request`.
fn run_worker(
    source: &[u8],
    request: HttpRequest,
) -> Result<JobResponse, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(source, directory.path())?;
    let (response, responses) = flume::bounded(1);
    let job = Job {
        cancelled: CancellationToken::new(),
        deadline: None,
        request,
        response,
        websocket: None,
    };
    let tokio = tokio::runtime::Runtime::new()?;
    tokio.block_on(tokio.spawn_blocking(move || {
        execute_request(&worker, &runtime_config(), job, &CancellationToken::new())
    }))??;
    Ok(responses.recv()?)
}

#[test]
#[allow(clippy::too_many_lines)]
fn exposes_bundle_tmp_and_device_operations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("config"))?;
    std::fs::write(
        directory.path().join("config/app.json"),
        br#"{"enabled":true}"#,
    )?;
    let runtime = JsRuntime::new()?;
    configure_worker_loader(
        &runtime,
        &WorkerBundle::new(
            WorkerManifest::es_modules("entry.js", &[]),
            PackageLayout::new(directory.path()),
        ),
    );
    let context = Context::full(&runtime)?;
    context.with(|ctx| {
        let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(crate::fs::Bundle::new(
            directory.path(),
        ))));
        crate::fs::install(&ctx, &vfs)
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?;
        crate::compat::initialize(&ctx)
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?;
        let object: Object = Module::import(&ctx, "node:fs")
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?
            .finish()
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?;
        let read: Function = object
            .get("readFileSync")
            .map_err(|error| Error::Engine(format!("readFileSync: {error}")))?;
        let value: TypedArray<u8> = read
            .call(("/bundle/config/app.json",))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(
            value
                .as_bytes()
                .ok_or_else(|| Error::Engine("detached bundle bytes".to_owned()))?,
            br#"{"enabled":true}"#
        );
        let write: Function = object
            .get("writeFileSync")
            .map_err(|error| Error::Engine(format!("writeFileSync: {error}")))?;
        let data = TypedArray::<u8>::new_copy(ctx.clone(), b"value")
            .map_err(|error| Error::Engine(error.to_string()))?;
        write
            .call::<_, ()>(("/tmp/value.txt", data))
            .map_err(|error| Error::Engine(error.to_string()))?;
        let info: Function = object
            .get("statSync")
            .map_err(|error| Error::Engine(format!("statSync: {error}")))?;
        let value: Object = info
            .call(("/tmp/value.txt", true))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(
            value
                .get::<_, u64>("size")
                .map_err(|error| Error::Engine(error.to_string()))?,
            5
        );

        let open: Function = object
            .get("openSync")
            .map_err(|error| Error::Engine(format!("openSync: {error}")))?;
        let descriptor: u32 = open
            .call(("/dev/zero", "r"))
            .map_err(|error| Error::Engine(error.to_string()))?;
        let read: Function = object
            .get("readSync")
            .map_err(|error| Error::Engine(format!("readSync: {error}")))?;
        let zeros = TypedArray::<u8>::new_copy(ctx.clone(), [0_u8; 3])
            .map_err(|error| Error::Engine(error.to_string()))?;
        let read_count: u32 = read
            .call((descriptor, zeros.clone(), 0_u32, 3_u32, 0_i64))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(read_count, 3);
        assert_eq!(zeros.as_bytes(), Some(&[0, 0, 0][..]));
        let device_info: Object = info
            .call(("/dev/zero", true))
            .map_err(|error| Error::Engine(error.to_string()))?;
        let is_character_device: Function = device_info
            .get("isCharacterDevice")
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert!(
            is_character_device
                .call::<_, bool>((rquickjs::function::This(device_info),))
                .map_err(|error| Error::Engine(error.to_string()))?
        );
        let close: Function = object
            .get("closeSync")
            .map_err(|error| Error::Engine(format!("closeSync: {error}")))?;
        close
            .call::<_, ()>((descriptor,))
            .map_err(|error| Error::Engine(error.to_string()))?;
        Ok::<_, Error>(())
    })?;
    Ok(())
}

#[test]
fn assembles_fragmented_text_and_binary_websocket_messages()
-> Result<(), Box<dyn std::error::Error>> {
    let (incoming, incoming_receiver) = flume::bounded(2);
    let mut fragmented = None;

    queue_websocket_message(&incoming, &mut fragmented, false, 0x1, b"ping ".to_vec())?;
    queue_websocket_message(&incoming, &mut fragmented, true, 0x0, b"42".to_vec())?;
    queue_websocket_message(&incoming, &mut fragmented, true, 0x2, vec![1, 2, 3])?;

    let None = fragmented else {
        panic!("a fragmented message was left unfinished");
    };
    let WebSocketInbound::Message {
        binary: false,
        payload,
    } = incoming_receiver.recv()?
    else {
        panic!("the fragmented message was not text");
    };
    assert_eq!(payload, b"ping 42");
    let WebSocketInbound::Message {
        binary: true,
        payload,
    } = incoming_receiver.recv()?
    else {
        panic!("the binary message was not binary");
    };
    assert_eq!(payload, vec![1, 2, 3]);
    Ok(())
}

#[test]
fn request_tasks_run_concurrently_on_tokio_blocking_workers()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()?;
    let (started_sender, started_receiver) = flume::bounded(2);
    let (first_release_sender, first_release_receiver) = flume::bounded(1);
    let (second_release_sender, second_release_receiver) = flume::bounded(1);

    let first = spawn_blocking_request(&tokio, started_sender.clone(), first_release_receiver);
    let second = spawn_blocking_request(&tokio, started_sender, second_release_receiver);

    let first_started = started_receiver.recv_timeout(Duration::from_secs(1));
    let second_started = started_receiver.recv_timeout(Duration::from_secs(1));
    first_release_sender.send(())?;
    second_release_sender.send(())?;
    tokio.block_on(async {
        first.await??;
        second.await??;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })?;

    first_started?;
    second_started?;
    Ok(())
}

pub(crate) fn certificates(directory: &Path) -> io::Result<Arc<Certificates>> {
    Certificates::start(directory.join("state"), HOST.to_owned(), None)
        .map(Arc::new)
        .map_err(io::Error::other)
}

fn spawn_blocking_request(
    tokio: &tokio::runtime::Runtime,
    started: Sender<()>,
    release: Receiver<()>,
) -> tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> {
    tokio.spawn_blocking(move || {
        started.send(())?;
        release.recv_timeout(Duration::from_secs(1))?;
        Ok(())
    })
}

pub(crate) fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        assets: None,
        cache: Arc::default(),
        services: None,
        environment: BTreeMap::new(),
        storage: None,
        plugins: Some(crate::plugin_calls::PluginCalls::new(None, true)),
    }
}

pub(crate) fn native_completion() -> crate::PluginReply {
    crate::PluginReply {
        payload: b"null".to_vec(),
        failed: false,
        done: true,
    }
}
