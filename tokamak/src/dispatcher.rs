use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use crate::compat;
use crate::fs::VirtualFileSystem;
use crate::fs::{
    MODULE_NAME as NODE_FS_MODULE_NAME, NodeFsModule, NodeFsPromisesModule,
    PROMISES_MODULE_NAME as NODE_FS_PROMISES_MODULE_NAME, install,
};
use crate::gateway::{
    Execution, Handler, Job, JobResponse, WebSocketInbound, WebSocketJob, WebSocketOutbound,
};
use crate::quickjs::{Assets, Error, RuntimeConfig, WorkerBundle};
use crate::transport::{BodyChunk, HttpRequest, HttpResponse, response_stream};
use flate2::read::GzDecoder;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{
    Array, ArrayBuffer, Context, Ctx, Function, Module, Object, Promise, Runtime as JsRuntime,
    TypedArray, Value,
};

/// Dispatches packaged application requests through `QuickJS`.
pub(crate) struct Dispatcher {
    worker: Arc<WorkerBundle>,
    config: RuntimeConfig,
    assets: Option<Arc<AssetService>>,
}

impl Dispatcher {
    pub(crate) fn new(bundle: WorkerBundle, config: RuntimeConfig) -> Result<Arc<Self>, Error> {
        let assets = config
            .assets
            .as_ref()
            .map(AssetService::new)
            .transpose()?
            .map(Arc::new);
        Ok(Arc::new(Self {
            worker: Arc::new(bundle),
            config,
            assets,
        }))
    }
}

impl Handler for Dispatcher {
    fn handle(
        &self,
        job: Job,
        execution: &Execution<'_>,
        accepting: &Arc<AtomicBool>,
    ) -> Result<(), Error> {
        let response = job.response.clone();
        if let Some(assets) = &self.assets
            && let Some(asset) = assets.response(&job.request)?
        {
            response
                .send(JobResponse::Http(asset))
                .map_err(|_| Error::Startup("HTTP response receiver closed".to_owned()))?;
            return Ok(());
        }
        execute_request(
            &self.worker,
            &self.config,
            self.assets.as_ref(),
            job,
            execution,
            accepting,
        )
    }
}

pub(super) fn configure_worker_loader(runtime: &JsRuntime, worker: &WorkerBundle) {
    runtime.set_loader(
        WorkerResolver,
        WorkerLoader {
            bundle: worker.clone(),
        },
    );
}

pub(super) fn execute_request(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    assets: Option<&Arc<AssetService>>,
    job: Job,
    execution: &Execution<'_>,
    accepting: &Arc<AtomicBool>,
) -> Result<(), Error> {
    let runtime = JsRuntime::new().map_err(|error| js_error("runtime", error))?;
    let interrupt_accepting = Arc::clone(accepting);
    runtime.set_interrupt_handler(Some(Box::new(move || {
        !interrupt_accepting.load(Ordering::Acquire)
    })));
    configure_worker_loader(&runtime, worker);
    let context = Context::full(&runtime).map_err(|error| js_error("context", error))?;
    let result = context.with(|ctx| -> Result<(), Error> {
        let Job {
            request,
            response: response_sender,
            websocket,
        } = job;
        let environment = serde_json::to_string(&config.environment)?;
        let cache = serde_json::to_string(&config.cache.to_string_lossy().to_string())?;
        let descriptor = serde_json::to_string(&request)?;
        let body = request
            .body
            .as_deref()
            .map(|body| ArrayBuffer::new_copy(ctx.clone(), body))
            .transpose()
            .map_err(|error| js_error("request body", error))?;
        let setup = format!(
            "globalThis.__tokamak_env = {environment}; globalThis.__tokamak_cache = {cache}; globalThis.__tokamak_request = {descriptor};"
        );
        ctx.eval::<(), _>(setup)
            .map_err(|error| js_error("setup", error))?;
        ctx.globals()
            .set("__tokamak_body", body)
            .map_err(|error| js_error("request body", error))?;
        if let Some(assets) = assets {
            install_asset_lookup(&ctx, assets)?;
        }

        initialize_worker_context(&ctx, worker)?;
        let fetch = load_worker(&ctx, worker)?;
        let request: Object = ctx
            .eval("new Request(__tokamak_request.url, { method: __tokamak_request.method, headers: __tokamak_request.headers, body: __tokamak_body ? new Uint8Array(__tokamak_body) : undefined })")
            .map_err(|error| js_error("request", error))?;
        let environment: Object = ctx
            .globals()
            .get("__tokamak_env")
            .map_err(|error| js_error("environment", error))?;
        let execution_context: Object = ctx
            .globals().get("__tokamak_context")
            .map_err(|error| js_error("execution context", error))?;
        let response: Promise = fetch
            .call((request, environment, execution_context.clone()))
            .map_err(|error| js_error("fetch", error))?;
        let response: Object = finish_promise(&ctx, &response, "response")?;
        let web_socket: Option<Object> = response
            .get("webSocket")
            .map_err(|error| js_error("response WebSocket", error))?;
        let response = response_from_js(&ctx, &response)?;
        if response.status == 101
            && let (Some(web_socket), Some(websocket)) = (web_socket, websocket)
        {
            let server: Object = web_socket
                .get("__tokamak_peer")
                .map_err(|error| js_error("WebSocket peer", error))?;
            let receive: Function = server
                .get("__tokamak_receive")
                .map_err(|error| js_error("WebSocket receive", error))?;
            let close: Function = server
                .get("__tokamak_close")
                .map_err(|error| js_error("WebSocket close", error))?;
            response_sender
                .send(JobResponse::WebSocket)
                .map_err(|_| Error::Startup("WebSocket response receiver closed".to_owned()))?;
            websocket_loop(
                &ctx,
                &web_socket,
                &receive,
                &close,
                &websocket,
                execution,
            )?;
            return drain_wait_until(&ctx, &execution_context);
        }
        send_worker_response(&ctx, response, &response_sender, &execution_context)
    });
    crate::network::cleanup_thread_sockets();
    result
}

fn initialize_worker_context(ctx: &Ctx<'_>, worker: &WorkerBundle) -> Result<(), Error> {
    let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(
        worker.vfs_bundle.clone(),
    )));
    install(ctx, &vfs).map_err(|error| js_error("node fs", error))?;
    compat::initialize(ctx).map_err(|error| js_exception(ctx, "runtime initialization", error))
}

pub(super) fn load_worker<'js>(
    ctx: &rquickjs::Ctx<'js>,
    bundle: &WorkerBundle,
) -> Result<Function<'js>, Error> {
    let bytes = read_worker_module(bundle, &bundle.entry)?;
    let module = unsafe { Module::load(ctx.clone(), &bytes) }
        .map_err(|error| js_exception(ctx, "load", error))?;
    let (module, evaluation) = module.eval().map_err(|error| js_error("evaluate", error))?;
    finish_promise::<()>(ctx, &evaluation, "module initialization")?;
    let exports = module
        .namespace()
        .map_err(|error| js_error("worker exports", error))?;
    ctx.globals()
        .set("__tokamak_exports", exports.clone())
        .map_err(|error| js_error("worker exports", error))?;
    let execution_context: Value = ctx
        .globals()
        .get("__tokamak_context")
        .map_err(|error| js_error("execution context", error))?;
    if let Ok(context) = Object::from_value(execution_context.clone()) {
        context
            .set("exports", exports.clone())
            .map_err(|error| js_error("execution context exports", error))?;
    }
    let environment: Value = ctx
        .globals()
        .get("__tokamak_env")
        .map_err(|error| js_error("environment", error))?;
    let default: Value = exports
        .get("default")
        .map_err(|error| js_error("worker export", error))?;
    let instantiate: Function = ctx
        .eval("(worker, context, env) => typeof worker === 'function' ? new worker(context, env) : worker")
        .map_err(|error| js_error("worker entrypoint", error))?;
    let worker: Object = instantiate
        .call((default, execution_context, environment))
        .map_err(|error| js_error("worker entrypoint", error))?;
    let fetch: Function = worker
        .get("fetch")
        .map_err(|error| js_error("worker fetch", error))?;
    let invoke: Function = ctx
        .eval("(fetch, worker) => (...args) => Promise.resolve(Reflect.apply(fetch, worker, args))")
        .map_err(|error| js_error("worker fetch", error))?;
    invoke
        .call((fetch, worker))
        .map_err(|error| js_error("worker fetch", error))
}

pub(super) struct WorkerResolver;

impl Resolver for WorkerResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &rquickjs::Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if let Some(builtin) = crate::runtime_modules::public_module(name) {
            return Ok(builtin.to_owned());
        }
        // Runtime implementation modules are not part of the application API.
        if name.starts_with("tokamak:") {
            return if base.starts_with("tokamak:") || (base.is_empty() && name == compat::BOOTSTRAP)
            {
                Ok(name.to_owned())
            } else {
                Err(rquickjs::Error::new_resolving(base, name))
            };
        }
        let resolved = if name.starts_with('.')
            && let Some(base) = base.strip_prefix("tokamak:")
        {
            format!(
                "tokamak:{}",
                crate::compiler::resolve_module_name(base, name)
            )
        } else {
            crate::compiler::resolve_module_name(base, name)
        };
        if is_module_name(&resolved)
            && (!resolved.starts_with("tokamak:") || base.starts_with("tokamak:"))
        {
            Ok(resolved)
        } else {
            Err(rquickjs::Error::new_resolving(base, name))
        }
    }
}

pub(super) struct WorkerLoader {
    pub(super) bundle: WorkerBundle,
}

impl Loader for WorkerLoader {
    fn load<'js>(
        &mut self,
        ctx: &rquickjs::Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        match name {
            NODE_FS_MODULE_NAME => {
                return Module::declare_def::<NodeFsModule, _>(ctx.clone(), name);
            }
            NODE_FS_PROMISES_MODULE_NAME => {
                return Module::declare_def::<NodeFsPromisesModule, _>(ctx.clone(), name);
            }
            "tokamak:host" => {
                return Module::declare_def::<crate::globals::native::HostModule, _>(
                    ctx.clone(),
                    name,
                );
            }
            _ => {}
        }
        if let Some(bytecode) = compat::bytecode(name) {
            // Builtin bytecode is compiled with the runtime during its build.
            return unsafe { Module::load(ctx.clone(), bytecode) };
        }
        if name.contains(':') {
            return Err(rquickjs::Error::new_loading(name));
        }
        let bytes = read_worker_module(&self.bundle, name)
            .map_err(|error| rquickjs::Error::new_loading_message(name, error.to_string()))?;
        // Packaged bytecode is produced by tokamak itself and is trusted here.
        unsafe { Module::load(ctx.clone(), &bytes) }
    }
}

fn is_module_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('/')
        && !name.contains('\\')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn read_worker_module(bundle: &WorkerBundle, name: &str) -> io::Result<Vec<u8>> {
    if !is_module_name(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Worker module name",
        ));
    }
    if let Some(bytecode) = &bundle.legacy {
        if name == bundle.entry {
            return Ok((**bytecode).clone());
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Worker module not found",
        ));
    }
    let bytes = std::fs::read(bundle.modules.join(format!("{name}.qjs")))?;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(bytes);
    }
    let mut decoder = GzDecoder::new(bytes.as_slice());
    let mut bytecode = Vec::new();
    decoder.read_to_end(&mut bytecode)?;
    Ok(bytecode)
}

fn websocket_loop<'js>(
    ctx: &rquickjs::Ctx<'js>,
    client: &Object<'js>,
    receive: &Function<'js>,
    close: &Function<'js>,
    websocket: &WebSocketJob,
    execution: &Execution<'_>,
) -> Result<(), Error> {
    let take_outbox: Function = ctx
        .eval(
            "socket => { const outbox = socket.__tokamak_outbox; socket.__tokamak_outbox = []; return outbox; }",
        )
        .map_err(|error| js_error("WebSocket outbox", error))?;
    drain_pending_jobs(ctx);
    drain_websocket_outbox(client, &take_outbox, &websocket.outgoing)?;
    signal_websocket_ready(&websocket.outgoing)?;

    loop {
        if !execution.is_running() {
            return Ok(());
        }
        let event = match websocket.incoming.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        };
        let should_close = match event {
            WebSocketInbound::Message { binary, payload } => {
                if binary {
                    let data = ArrayBuffer::new_copy(ctx.clone(), payload)
                        .map_err(|error| js_error("WebSocket message", error))?;
                    receive
                        .call::<_, ()>((data, true))
                        .map_err(|error| js_exception(ctx, "WebSocket message", error))?;
                } else {
                    let data = String::from_utf8_lossy(&payload).into_owned();
                    receive
                        .call::<_, ()>((data, false))
                        .map_err(|error| js_exception(ctx, "WebSocket message", error))?;
                }
                false
            }
            WebSocketInbound::Close { code, reason } => {
                close
                    .call::<_, ()>((code, reason))
                    .map_err(|error| js_exception(ctx, "WebSocket close", error))?;
                true
            }
        };
        drain_pending_jobs(ctx);
        drain_websocket_outbox(client, &take_outbox, &websocket.outgoing)?;
        signal_websocket_ready(&websocket.outgoing)?;
        if should_close {
            return Ok(());
        }
    }
}

fn drain_pending_jobs(ctx: &rquickjs::Ctx<'_>) {
    while ctx.execute_pending_job() {}
}

fn drain_websocket_outbox<'js>(
    client: &Object<'js>,
    take_outbox: &Function<'js>,
    outgoing: &SyncSender<WebSocketOutbound>,
) -> Result<(), Error> {
    let messages: Array = take_outbox
        .call((client.clone(),))
        .map_err(|error| js_error("WebSocket outbox", error))?;
    for entry in messages.iter::<Object>() {
        let entry = entry.map_err(|error| js_error("WebSocket outbox entry", error))?;
        let message_type: String = entry
            .get("type")
            .map_err(|error| js_error("WebSocket outbox type", error))?;
        match message_type.as_str() {
            "message" => {
                let binary: bool = entry
                    .get("binary")
                    .map_err(|error| js_error("WebSocket outbox binary flag", error))?;
                let payload = if binary {
                    let data: ArrayBuffer = entry
                        .get("data")
                        .map_err(|error| js_error("WebSocket outbox data", error))?;
                    data.as_bytes()
                        .ok_or_else(|| Error::Engine("WebSocket data was detached".to_owned()))?
                        .to_vec()
                } else {
                    let data: String = entry
                        .get("data")
                        .map_err(|error| js_error("WebSocket outbox data", error))?;
                    data.into_bytes()
                };
                outgoing
                    .send(WebSocketOutbound::Message { binary, payload })
                    .map_err(|_| Error::Startup("WebSocket connection closed".to_owned()))?;
            }
            "close" => {
                let code: u16 = entry
                    .get("code")
                    .map_err(|error| js_error("WebSocket close code", error))?;
                let reason: String = entry
                    .get("reason")
                    .map_err(|error| js_error("WebSocket close reason", error))?;
                outgoing
                    .send(WebSocketOutbound::Close { code, reason })
                    .map_err(|_| Error::Startup("WebSocket connection closed".to_owned()))?;
            }
            _ => return Err(Error::Engine("unknown WebSocket outbox entry".to_owned())),
        }
    }
    Ok(())
}

fn signal_websocket_ready(outgoing: &SyncSender<WebSocketOutbound>) -> Result<(), Error> {
    outgoing
        .send(WebSocketOutbound::Ready)
        .map_err(|_| Error::Startup("WebSocket connection closed".to_owned()))
}

fn drain_wait_until<'js>(
    ctx: &rquickjs::Ctx<'js>,
    _execution_context: &Object<'js>,
) -> Result<(), Error> {
    let drain: Function = ctx
        .globals()
        .get("__tokamak_drain_wait_until")
        .map_err(|error| js_error("waitUntil", error))?;
    let pending: Promise = drain
        .call(())
        .map_err(|error| js_error("waitUntil", error))?;
    finish_promise::<()>(ctx, &pending, "waitUntil")?;
    Ok(())
}

struct JsResponse<'js> {
    status: u16,
    headers: BTreeMap<String, String>,
    body: JsResponseBody<'js>,
}

enum JsResponseBody<'js> {
    Buffered(Vec<u8>),
    Stream(Object<'js>),
}

fn response_from_js<'js>(
    ctx: &rquickjs::Ctx<'js>,
    response: &Object<'js>,
) -> Result<JsResponse<'js>, Error> {
    let status: u16 = response
        .get("status")
        .map_err(|error| js_error("response status", error))?;
    let mut headers = BTreeMap::new();
    let object: Object = response
        .get("headers")
        .map_err(|error| js_error("response headers", error))?;
    let entries_fn: Function = ctx
        .eval("headers => Array.from(headers)")
        .map_err(|error| js_error("response headers", error))?;
    let entries: Array = entries_fn
        .call((object,))
        .map_err(|error| js_error("response headers", error))?;
    for entry in entries.iter::<Array>() {
        let entry = entry.map_err(|error| js_error("response header", error))?;
        let name: String = entry
            .get(0)
            .map_err(|error| js_error("response header name", error))?;
        let value: String = entry
            .get(1)
            .map_err(|error| js_error("response header value", error))?;
        headers.insert(name, value);
    }
    let stream: Value = response
        .get("__stream")
        .map_err(|error| js_error("response body", error))?;
    let body = if stream.is_null() || stream.is_undefined() {
        JsResponseBody::Buffered(buffered_response_body(ctx, response)?)
    } else {
        JsResponseBody::Stream(
            Object::from_value(stream).map_err(|error| js_error("response stream", error))?,
        )
    };
    Ok(JsResponse {
        status,
        headers,
        body,
    })
}

fn buffered_response_body<'js>(
    ctx: &rquickjs::Ctx<'js>,
    response: &Object<'js>,
) -> Result<Vec<u8>, Error> {
    let value: Value = response
        .get("__body")
        .map_err(|error| js_error("response body", error))?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Ok(body) = TypedArray::<u8>::from_value(value.clone()) {
        return body
            .as_bytes()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::Engine("response body was detached".to_owned()));
    }
    if let Some(body) = ArrayBuffer::from_value(value) {
        return body
            .as_bytes()
            .map(ToOwned::to_owned)
            .ok_or_else(|| Error::Engine("response body was detached".to_owned()));
    }
    read_response_text(ctx, response)
}

fn read_response_text<'js>(
    ctx: &rquickjs::Ctx<'js>,
    response: &Object<'js>,
) -> Result<Vec<u8>, Error> {
    let read_body: Function = ctx
        .eval("response => response.text()")
        .map_err(|error| js_error("response body", error))?;
    let body: Promise = read_body
        .call((response.clone(),))
        .map_err(|error| js_error("response body", error))?;
    let body: String = finish_promise(ctx, &body, "response body")?;
    Ok(body.into_bytes())
}

fn send_worker_response<'js>(
    ctx: &rquickjs::Ctx<'js>,
    response: JsResponse<'js>,
    response_sender: &std::sync::mpsc::SyncSender<JobResponse>,
    execution_context: &Object<'js>,
) -> Result<(), Error> {
    let JsResponse {
        status,
        headers,
        body,
    } = response;
    match body {
        JsResponseBody::Buffered(body) => {
            response_sender
                .send(JobResponse::Http(HttpResponse::buffered(
                    status, headers, body,
                )))
                .map_err(|_| Error::Startup("HTTP response receiver closed".to_owned()))?;
            drain_wait_until(ctx, execution_context)
        }
        JsResponseBody::Stream(stream) => {
            let (sender, cancelled, body) = response_stream();
            response_sender
                .send(JobResponse::Http(HttpResponse {
                    status,
                    headers,
                    body,
                }))
                .map_err(|_| Error::Startup("HTTP response receiver closed".to_owned()))?;
            pump_response_stream(ctx, stream, &sender, &cancelled);
            if let Err(error) = drain_wait_until(ctx, execution_context) {
                eprintln!("Worker waitUntil failed after streaming response: {error}");
            }
            Ok(())
        }
    }
}

fn pump_response_stream<'js>(
    ctx: &rquickjs::Ctx<'js>,
    stream: Object<'js>,
    sender: &std::sync::mpsc::SyncSender<BodyChunk>,
    cancelled: &AtomicBool,
) {
    let result = pump_response_stream_inner(ctx, stream, sender, cancelled);
    if let Err(error) = result {
        let _ = sender.send(Err(error.to_string()));
    }
}

fn pump_response_stream_inner<'js>(
    ctx: &rquickjs::Ctx<'js>,
    stream: Object<'js>,
    sender: &std::sync::mpsc::SyncSender<BodyChunk>,
    cancelled: &AtomicBool,
) -> Result<(), Error> {
    let get_reader: Function = ctx
        .eval("stream => stream.getReader()")
        .map_err(|error| js_error("response stream reader", error))?;
    let reader: Object = get_reader
        .call((stream,))
        .map_err(|error| js_error("response stream reader", error))?;
    let read: Function = ctx
        .eval("reader => reader.read()")
        .map_err(|error| js_error("response stream read", error))?;
    let cancel: Function = ctx
        .eval("reader => reader.cancel()")
        .map_err(|error| js_error("response stream cancel", error))?;
    let to_bytes: Function = ctx
        .eval(
            "value => {\
                if (typeof value === 'string') return new TextEncoder().encode(value);\
                if (value instanceof Uint8Array) return value;\
                if (value instanceof ArrayBuffer) return new Uint8Array(value);\
                if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);\
                throw new TypeError('response stream chunks must be byte-oriented');\
            }",
        )
        .map_err(|error| js_error("response stream chunk", error))?;
    loop {
        if cancelled.load(Ordering::Acquire) {
            cancel_response_stream(ctx, &cancel, &reader);
            return Ok(());
        }
        let pending: Promise = read
            .call((reader.clone(),))
            .map_err(|error| js_error("response stream read", error))?;
        let result: Object = finish_promise(ctx, &pending, "response stream read")?;
        let done: bool = result
            .get("done")
            .map_err(|error| js_error("response stream result", error))?;
        if done {
            return Ok(());
        }
        let value: Value = result
            .get("value")
            .map_err(|error| js_error("response stream result", error))?;
        let chunk: TypedArray<u8> = to_bytes
            .call((value,))
            .map_err(|error| js_exception(ctx, "response stream chunk", error))?;
        let Some(bytes) = chunk.as_bytes() else {
            return Err(Error::Engine(
                "response stream chunk was detached".to_owned(),
            ));
        };
        if sender.send(Ok(bytes.to_vec())).is_err() {
            cancel_response_stream(ctx, &cancel, &reader);
            return Ok(());
        }
    }
}

fn cancel_response_stream<'js>(
    ctx: &rquickjs::Ctx<'js>,
    cancel: &Function<'js>,
    reader: &Object<'js>,
) {
    let Ok(pending) = cancel.call::<_, Promise>((reader.clone(),)) else {
        return;
    };
    let _ = finish_promise::<Value>(ctx, &pending, "response stream cancel");
}

fn finish_promise<'js, T: rquickjs::FromJs<'js>>(
    ctx: &rquickjs::Ctx<'js>,
    promise: &Promise<'js>,
    stage: &str,
) -> Result<T, Error> {
    let timers: Option<Function> = ctx
        .globals()
        .get("__tokamak_run_timers")
        .map_err(|error| js_error(stage, error))?;
    loop {
        if let Some(result) = promise.result::<T>() {
            return result.map_err(|error| js_exception(ctx, stage, error));
        }
        if ctx.execute_pending_job() {
            continue;
        }
        let Some(timers) = &timers else {
            return Err(js_error(stage, "promise did not settle"));
        };
        let poll: Array = timers
            .call(())
            .map_err(|error| js_exception(ctx, stage, error))?;
        let ran: bool = poll
            .get(0)
            .map_err(|error| js_exception(ctx, stage, error))?;
        let wait: Option<u64> = poll
            .get(1)
            .map_err(|error| js_exception(ctx, stage, error))?;
        if ran {
            continue;
        }
        let Some(wait) = wait else {
            return Err(js_error(stage, "promise did not settle"));
        };
        std::thread::sleep(Duration::from_millis(wait.clamp(1, 50)));
    }
}

/// Serves static assets from a manifest parsed once at startup.
pub(super) struct AssetService {
    root: std::path::PathBuf,
    manifest: AssetManifest,
}

impl AssetService {
    pub(super) fn new(assets: &Assets) -> Result<Self, Error> {
        let manifest = serde_json::from_slice(&std::fs::read(&assets.manifest)?)?;
        Ok(Self {
            root: assets.root.clone(),
            manifest,
        })
    }

    pub(super) fn response(&self, request: &HttpRequest) -> Result<Option<HttpResponse>, Error> {
        if request.method != "GET" && request.method != "HEAD" {
            return Ok(None);
        }
        let path = request.target.split('?').next().unwrap_or("/");
        let Some(relative) = self.manifest.path_for(path) else {
            return Ok(None);
        };
        let body = std::fs::read(self.root.join(&relative))?;
        let mut response = HttpResponse::buffered(
            200,
            BTreeMap::new(),
            if request.method == "HEAD" {
                Vec::new()
            } else {
                body
            },
        );
        response
            .headers
            .insert("content-type".to_owned(), self.manifest.content_type(&relative));
        Ok(Some(response))
    }
}

fn install_asset_lookup<'js>(ctx: &Ctx<'js>, service: &Arc<AssetService>) -> Result<(), Error> {
    let service = Arc::clone(service);
    let lookup = Function::new(
        ctx.clone(),
        rquickjs::function::MutFn::new(move |ctx: Ctx<'js>, path: String| {
            asset_lookup(ctx, &service, &path)
        }),
    )
    .map_err(|error| js_error("asset lookup", error))?;
    ctx.globals()
        .set("__tokamak_asset", lookup)
        .map_err(|error| js_error("asset lookup", error))
}

fn asset_lookup<'js>(
    ctx: Ctx<'js>,
    service: &AssetService,
    path: &str,
) -> rquickjs::Result<Option<Object<'js>>> {
    let path = path.split('?').next().unwrap_or("/");
    let Some(relative) = service.manifest.path_for(path) else {
        return Ok(None);
    };
    let Ok(body) = std::fs::read(service.root.join(&relative)) else {
        return Ok(None);
    };
    let result = Object::new(ctx.clone())?;
    result.set("contentType", service.manifest.content_type(&relative))?;
    result.set("body", TypedArray::new(ctx, body)?)?;
    Ok(Some(result))
}

#[derive(serde::Deserialize)]
pub(super) struct AssetManifest {
    pub(super) files: BTreeMap<String, String>,
    #[serde(rename = "htmlHandling")]
    pub(super) html_handling: String,
}

impl AssetManifest {
    pub(super) fn path_for(&self, path: &str) -> Option<String> {
        let path = path.trim_start_matches('/');
        let candidates = match self.html_handling.as_str() {
            "force-trailing-slash" | "auto-trailing-slash" => {
                if path.is_empty() {
                    vec!["index.html".to_owned()]
                } else if path.ends_with('/') {
                    vec![
                        format!("{path}index.html"),
                        path.trim_end_matches('/').to_owned(),
                    ]
                } else {
                    vec![format!("{path}/index.html"), path.to_owned()]
                }
            }
            "drop-trailing-slash" => vec![
                path.trim_end_matches('/').to_owned(),
                format!("{path}.html"),
            ],
            _ => vec![path.to_owned()],
        };
        candidates.into_iter().find(|candidate| {
            self.files.contains_key(&format!("/{candidate}")) || self.files.contains_key(candidate)
        })
    }

    pub(super) fn content_type(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        self.files
            .get(path)
            .or_else(|| self.files.get(&format!("/{path}")))
            .cloned()
            .unwrap_or_else(|| "application/octet-stream".to_owned())
    }
}

fn js_error(stage: &str, error: impl std::fmt::Display) -> Error {
    Error::Engine(format!("{stage}: {error}"))
}

fn js_exception(ctx: &rquickjs::Ctx<'_>, stage: &str, error: impl std::fmt::Display) -> Error {
    let value = ctx.catch();
    let detail = value.as_exception().map_or_else(
        || error.to_string(),
        |exception| {
            let message = exception.message().unwrap_or_default();
            let stack = exception.stack().unwrap_or_default();
            if stack.is_empty() {
                message
            } else {
                format!("{message}\n{stack}")
            }
        },
    );
    Error::Engine(format!("{stage}: {detail}"))
}
