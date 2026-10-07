use flume::Sender;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::assets::{AssetResponse, Assets};
use crate::compat;
use crate::fs::VirtualFileSystem;
use crate::fs::{
    MODULE_NAME as NODE_FS_MODULE_NAME, NodeFsModule, NodeFsPromisesModule,
    PROMISES_MODULE_NAME as NODE_FS_PROMISES_MODULE_NAME, install, read_bundle_file,
};
use crate::gateway::{
    Handler, HandlerError, Job, JobResponse, WebSocketInbound, WebSocketJob, WebSocketOutbound,
    WebSocketOutgoing,
};
use crate::globals::ResponseEncoder;
use crate::linked::StorageRuntime;
use crate::packaging::{self, ModuleType};
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::transport::{BodyChunk, HttpRequest, HttpResponse, append_header, response_stream};
use hyper::header::HeaderMap;
use rquickjs::convert::List;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    Array, ArrayBuffer, AsyncContext, AsyncRuntime, Ctx, Function, Module, Object, Promise,
    TypedArray, Value,
};

/// Dispatches packaged application requests to the app's assets or, through
/// `QuickJS`, its Worker.
pub(crate) struct Dispatcher {
    worker: Arc<WorkerBundle>,
    config: RuntimeConfig,
}

impl Dispatcher {
    pub(crate) fn new(bundle: WorkerBundle, config: RuntimeConfig) -> Arc<Self> {
        Arc::new(Self {
            worker: Arc::new(bundle),
            config,
        })
    }
}

impl Handler for Dispatcher {
    fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError> {
        if let Some(assets) = &self.config.assets
            && let Some(asset) = assets.route(&job.request)?
        {
            job.response
                .send(JobResponse::Http(asset))
                .map_err(|_| receiver_closed("HTTP response receiver closed"))?;
            return Ok(());
        }
        execute_request(&self.worker, &self.config, job, stopped).map_err(Into::into)
    }
}

#[cfg(test)]
pub(super) fn configure_worker_loader(runtime: &rquickjs::Runtime, worker: &WorkerBundle) {
    runtime.set_loader(
        WorkerResolver,
        WorkerLoader {
            bundle: worker.clone(),
            storage: None,
        },
    );
}

/// Runs `job` on the Worker from a blocking thread of the gateway's Tokio runtime.
pub(super) fn execute_request(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    job: Job,
    stopped: &CancellationToken,
) -> Result<(), Error> {
    tokio::runtime::Handle::try_current()
        .map_err(io::Error::other)?
        .block_on(execute_request_async(worker, config, job, stopped))
}

/// A fresh `QuickJS` runtime that loads the Worker's modules and is
/// interrupted once `stopped` is cancelled.
async fn worker_runtime(
    worker: &WorkerBundle,
    storage: Option<&Arc<dyn StorageRuntime>>,
    stopped: &CancellationToken,
) -> Result<(AsyncRuntime, AsyncContext), Error> {
    let runtime = AsyncRuntime::new().map_err(|error| js_error("runtime", error))?;
    let interrupted = stopped.clone();
    runtime
        .set_interrupt_handler(Some(Box::new(move || interrupted.is_cancelled())))
        .await;
    runtime
        .set_loader(
            WorkerResolver,
            WorkerLoader {
                bundle: worker.clone(),
                storage: storage.cloned(),
            },
        )
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|error| js_error("context", error))?;
    Ok((runtime, context))
}

async fn execute_request_async(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    job: Job,
    stopped: &CancellationToken,
) -> Result<(), Error> {
    let (runtime, context) = worker_runtime(worker, config.storage.as_ref(), stopped).await?;
    let awaited = crate::event_loop::AwaitedPromise::default();
    let request = context.async_with(async |ctx| -> Result<(), Error> {
        ctx.store_userdata(awaited.clone())
            .map_err(|error| js_error("request event loop", error))?;
        let Job {
            request,
            response: response_sender,
            websocket,
        } = job;
        install_worker_globals(&ctx, config)?;
        let descriptor = install_request(&ctx, &request)?;
        let bootstrap = initialize_worker_context(&ctx, worker)?;
        if let Some(assets) = &config.assets {
            install_assets(&ctx, &bootstrap, assets)
                .map_err(|error| js_exception(&ctx, "assets", error))?;
        }
        let response = invoke_worker(&ctx, worker, &bootstrap, descriptor, &request).await?;
        let response = response_from_js(&ctx, &bootstrap, response)?;
        if response.status == 101
            && let (Some(native), Some(websocket)) = (&response.web_socket, websocket)
        {
            response_sender
                .send_async(JobResponse::WebSocket)
                .await
                .map_err(|_| receiver_closed("WebSocket response receiver closed"))?;
            websocket_loop(&ctx, native, &websocket, stopped).await?;
            return drain_wait_until(&ctx).await;
        }
        send_worker_response(&ctx, response, &response_sender).await
    });
    let request = crate::event_loop::run(&runtime, &awaited, request);
    tokio::select! {
        result = request => result,
        () = stopped.cancelled() => Err(Error::Engine("Worker execution was stopped".to_owned())),
    }
}

async fn invoke_worker<'js>(
    ctx: &Ctx<'js>,
    worker: &WorkerBundle,
    bootstrap: &Object<'js>,
    descriptor: Value<'js>,
    request: &HttpRequest,
) -> Result<Value<'js>, Error> {
    let entrypoint = load_worker(ctx, worker).await?;
    let fetch = worker_fetch(ctx, &entrypoint)?;
    let body = request
        .body
        .as_deref()
        .map(|body| TypedArray::<u8>::new_copy(ctx.clone(), body))
        .transpose()
        .map_err(|error| js_error("request body", error))?;
    let request: Object = property::<Function>(bootstrap, "hostRequest")?
        .call((descriptor, body))
        .map_err(|error| js_exception(ctx, "request", error))?;
    let environment: Object = ctx
        .globals()
        .get("__tokamak_env")
        .map_err(|error| js_error("environment", error))?;
    let execution_context: Object = ctx
        .globals()
        .get("__tokamak_context")
        .map_err(|error| js_error("execution context", error))?;
    let response: Promise = fetch
        .call((request, environment, execution_context))
        .map_err(|error| js_error("fetch", error))?;
    finish_promise(ctx, &response, "response").await
}

fn install_worker_globals(ctx: &Ctx<'_>, config: &RuntimeConfig) -> Result<(), Error> {
    let environment = serde_json::to_string(&config.environment)?;
    ctx.eval::<(), _>(format!("globalThis.__tokamak_env = {environment};"))
        .map_err(|error| js_error("setup", error))?;
    crate::cache::attach(ctx, &config.cache).map_err(|error| js_error("caches", error))?;
    if let Some(storage) = &config.storage {
        Arc::clone(storage)
            .attach(ctx)
            .map_err(|error| js_error("storage", error))?;
    }
    Ok(())
}

/// Install the request's descriptor as `__tokamak_request` and return it.
fn install_request<'js>(ctx: &Ctx<'js>, request: &HttpRequest) -> Result<Value<'js>, Error> {
    let descriptor = ctx
        .json_parse(serde_json::to_string(request)?)
        .map_err(|error| js_error("setup", error))?;
    ctx.globals()
        .set("__tokamak_request", descriptor.clone())
        .map_err(|error| js_error("setup", error))?;
    Ok(descriptor)
}

/// Install the Node file system and evaluate the runtime bootstrap, returning
/// its exports.
fn initialize_worker_context<'js>(
    ctx: &Ctx<'js>,
    worker: &WorkerBundle,
) -> Result<Object<'js>, Error> {
    let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(
        worker.vfs_bundle.clone(),
    )));
    install(ctx, &vfs).map_err(|error| js_error("node fs", error))?;
    compat::initialize(ctx).map_err(|error| js_exception(ctx, "runtime initialization", error))
}

/// Evaluate the Worker's entry module and return its default export, constructed
/// when it is a class.
pub(super) async fn load_worker<'js>(
    ctx: &Ctx<'js>,
    bundle: &WorkerBundle,
) -> Result<Object<'js>, Error> {
    let bytes = read_worker_module(bundle, &bundle.entry)?;
    let module = unsafe { Module::load(ctx.clone(), &bytes) }
        .map_err(|error| js_exception(ctx, "load", error))?;
    let (module, evaluation) = module.eval().map_err(|error| js_error("evaluate", error))?;
    finish_promise::<()>(ctx, &evaluation, "module initialization").await?;
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
    if let Some(context) = execution_context.as_object() {
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
    let instantiate = eval_function(
        ctx,
        "(worker, context, env) => typeof worker === 'function' ? new worker(context, env) : worker",
        "worker entrypoint",
    )?;
    instantiate
        .call((default, execution_context, environment))
        .map_err(|error| js_error("worker entrypoint", error))
}

/// The entrypoint's `fetch` handler bound to it, returning a promise.
fn worker_fetch<'js>(ctx: &Ctx<'js>, entrypoint: &Object<'js>) -> Result<Function<'js>, Error> {
    let bind = eval_function(
        ctx,
        "(worker) => typeof worker.fetch === 'function' \
            ? (...args) => Promise.resolve(worker.fetch(...args)) \
            : undefined",
        "worker fetch",
    )?;
    let fetch: Option<Function> = bind
        .call((entrypoint.clone(),))
        .map_err(|error| js_error("worker fetch", error))?;
    fetch.ok_or_else(|| Error::Engine("worker fetch: the Worker does not export fetch".to_owned()))
}

pub(super) struct WorkerResolver;

impl Resolver for WorkerResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
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
    /// The stores the storage modules reach, when the app has storage
    /// bindings.
    pub(super) storage: Option<Arc<dyn StorageRuntime>>,
}

impl Loader for WorkerLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        match name {
            NODE_FS_MODULE_NAME => Module::declare_def::<NodeFsModule, _>(ctx.clone(), name),
            NODE_FS_PROMISES_MODULE_NAME => {
                Module::declare_def::<NodeFsPromisesModule, _>(ctx.clone(), name)
            }
            "tokamak:host" => {
                Module::declare_def::<crate::globals::native::HostModule, _>(ctx.clone(), name)
            }
            _ => {
                let storage = self.storage.as_ref();
                let module = storage.and_then(|storage| storage.module(ctx, name));
                module.unwrap_or_else(|| self.load_module(ctx, name))
            }
        }
    }
}

impl WorkerLoader {
    fn load_module<'js>(&self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
        if let Some(bytecode) = compat::bytecode(name) {
            // Builtin bytecode is compiled with the runtime during its build.
            return unsafe { Module::load(ctx.clone(), bytecode) };
        }
        let Some(module_type) = self.bundle.module_types.get(name) else {
            return Err(rquickjs::Error::new_loading(name));
        };
        match module_type {
            ModuleType::EsModule => self.load_bytecode(ctx, name),
            ModuleType::Text => Module::declare_def::<TextModule, _>(ctx.clone(), name),
            ModuleType::Data => Module::declare_def::<DataModule, _>(ctx.clone(), name),
        }
    }

    fn load_bytecode<'js>(&self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
        let bytes = read_worker_module(&self.bundle, name)
            .map_err(|error| rquickjs::Error::new_loading_message(name, error.to_string()))?;
        // Packaged bytecode is produced by tokamak itself and is trusted here.
        unsafe { Module::load(ctx.clone(), &bytes) }
    }
}

/// A Text module, whose default export is its file decoded as UTF-8.
struct TextModule;

impl ModuleDef for TextModule {
    fn declare(declarations: &Declarations<'_>) -> rquickjs::Result<()> {
        declarations.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let bytes = module_file(ctx, exports)?;
        let (text, _) = encoding_rs::UTF_8.decode_with_bom_removal(&bytes);
        exports.export("default", &*text)?;
        Ok(())
    }
}

/// A Data module, whose default export is its file as an `ArrayBuffer`.
struct DataModule;

impl ModuleDef for DataModule {
    fn declare(declarations: &Declarations<'_>) -> rquickjs::Result<()> {
        declarations.declare("default")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        let bytes = module_file(ctx, exports)?;
        exports.export("default", ArrayBuffer::new(ctx.clone(), bytes)?)?;
        Ok(())
    }
}

/// The contents of the `/bundle` file behind the module `exports` belongs to.
fn module_file<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<Vec<u8>> {
    read_bundle_file(ctx, &exports.module().name::<String>()?)
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
    packaging::read_worker_module(&bundle.app, name).map_err(io::Error::other)
}

/// Relays frames between the host's WebSocket and `native`, the runtime's end
/// of the Worker's socket.
async fn websocket_loop<'js>(
    ctx: &Ctx<'js>,
    native: &Object<'js>,
    websocket: &WebSocketJob,
    stopped: &CancellationToken,
) -> Result<(), Error> {
    let changed = Arc::new(tokio::sync::Notify::new());
    let notify = Arc::clone(&changed);
    let notify = Function::new(ctx.clone(), move || notify.notify_one())
        .map_err(|error| js_error("WebSocket notification", error))?;
    property::<Function>(native, "listen")?
        .call::<_, ()>((notify,))
        .map_err(|error| js_exception(ctx, "WebSocket", error))?;
    let receive: Function = property(native, "receive")?;
    let close: Function = property(native, "close")?;
    let take_outbox: Function = property(native, "take")?;
    drain_pending_jobs(ctx);
    drain_websocket_outbox(&take_outbox, &websocket.outgoing).await?;
    send_outbound(&websocket.outgoing, WebSocketOutbound::Ready).await?;

    loop {
        if stopped.is_cancelled() {
            return Ok(());
        }
        let event = tokio::select! {
            event = websocket.incoming.recv_async() => match event {
                Ok(event) => event,
                Err(_) => return Ok(()),
            },
            () = changed.notified() => {
                drain_websocket_outbox(&take_outbox, &websocket.outgoing).await?;
                continue;
            },
            () = stopped.cancelled() => return Ok(()),
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
        drain_websocket_outbox(&take_outbox, &websocket.outgoing).await?;
        send_outbound(&websocket.outgoing, WebSocketOutbound::Ready).await?;
        if should_close {
            return Ok(());
        }
    }
}

fn drain_pending_jobs(ctx: &Ctx<'_>) {
    while ctx.execute_pending_job() {}
}

async fn drain_websocket_outbox(
    take_outbox: &Function<'_>,
    outgoing: &WebSocketOutgoing,
) -> Result<(), Error> {
    let messages: Array = take_outbox
        .call(())
        .map_err(|error| js_error("WebSocket outbox", error))?;
    for entry in messages.iter::<Object>() {
        let entry = entry.map_err(|error| js_error("WebSocket outbox entry", error))?;
        let message_type: String = entry
            .get("type")
            .map_err(|error| js_error("WebSocket outbox type", error))?;
        let frame = match message_type.as_str() {
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
                WebSocketOutbound::Message { binary, payload }
            }
            "close" => {
                let code: u16 = entry
                    .get("code")
                    .map_err(|error| js_error("WebSocket close code", error))?;
                let reason: String = entry
                    .get("reason")
                    .map_err(|error| js_error("WebSocket close reason", error))?;
                WebSocketOutbound::Close { code, reason }
            }
            _ => return Err(Error::Engine("unknown WebSocket outbox entry".to_owned())),
        };
        send_outbound(outgoing, frame).await?;
    }
    Ok(())
}

async fn send_outbound(
    outgoing: &WebSocketOutgoing,
    frame: WebSocketOutbound,
) -> Result<(), Error> {
    outgoing
        .send_async(frame)
        .await
        .map_err(|_| receiver_closed("WebSocket connection closed").into())
}

fn receiver_closed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, message)
}

async fn drain_wait_until(ctx: &Ctx<'_>) -> Result<(), Error> {
    let drain: Function = ctx
        .globals()
        .get("__tokamak_drain_wait_until")
        .map_err(|error| js_error("waitUntil", error))?;
    let pending: Promise = drain
        .call(())
        .map_err(|error| js_error("waitUntil", error))?;
    finish_promise(ctx, &pending, "waitUntil").await
}

struct JsResponse<'js> {
    status: u16,
    status_text: String,
    headers: HeaderMap,
    encoder: Option<ResponseEncoder>,
    body: JsResponseBody<'js>,
    /// The runtime's end of the response's WebSocket.
    web_socket: Option<Object<'js>>,
}

enum JsResponseBody<'js> {
    Buffered(Vec<u8>),
    /// The runtime's reader of a body stream: `read()` resolves to each chunk's
    /// bytes, then null.
    Stream(Object<'js>),
}

/// The parts of the Worker's `response` the runtime's `hostResponse` gives the host.
fn response_from_js<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    response: Value<'js>,
) -> Result<JsResponse<'js>, Error> {
    let parts: Object = property::<Function>(bootstrap, "hostResponse")?
        .call((response,))
        .map_err(|error| js_exception(ctx, "response", error))?;
    let mut headers = HeaderMap::new();
    let entries: Array = property(&parts, "headers")?;
    for entry in entries.iter::<List<(String, String)>>() {
        let List((name, value)) = entry.map_err(|error| js_error("response header", error))?;
        append_header(&mut headers, &name, &value)?;
    }
    let encoding: String = property(&parts, "encodeBody")?;
    let encoder = if encoding == "manual" {
        None
    } else {
        headers
            .get("content-encoding")
            .and_then(|value| value.to_str().ok())
            .map(ResponseEncoder::new)
            .transpose()?
            .flatten()
    };
    let body: Value = property(&parts, "body")?;
    let body = match TypedArray::<u8>::from_value(body.clone()) {
        Ok(bytes) => JsResponseBody::Buffered(
            bytes
                .as_bytes()
                .ok_or_else(|| Error::Engine("response body was detached".to_owned()))?
                .to_vec(),
        ),
        Err(_) => JsResponseBody::Stream(
            body.into_object()
                .ok_or_else(|| Error::Engine("response body is not a stream".to_owned()))?,
        ),
    };
    Ok(JsResponse {
        status: property(&parts, "status")?,
        status_text: property(&parts, "statusText")?,
        headers,
        encoder,
        body,
        web_socket: property(&parts, "webSocket")?,
    })
}

async fn send_worker_response<'js>(
    ctx: &Ctx<'js>,
    response: JsResponse<'js>,
    response_sender: &Sender<JobResponse>,
) -> Result<(), Error> {
    let JsResponse {
        status,
        status_text,
        headers,
        mut encoder,
        body,
        ..
    } = response;
    // A caller that stopped waiting gets no response, and `waitUntil` work still finishes.
    match body {
        JsResponseBody::Buffered(body) if encoder.is_none() => {
            let mut response = HttpResponse::buffered(status, headers, body);
            response.status_text = status_text;
            let _ = response_sender
                .send_async(JobResponse::Http(response))
                .await;
            drain_wait_until(ctx).await
        }
        body => {
            let (sender, cancelled, output) = response_stream();
            let _ = response_sender
                .send_async(JobResponse::Http(HttpResponse {
                    status,
                    status_text,
                    headers,
                    body: output,
                }))
                .await;
            let result = match body {
                JsResponseBody::Buffered(bytes) => {
                    send_response_chunk(&bytes, true, &mut encoder, &sender).await
                }
                JsResponseBody::Stream(reader) => {
                    pump_response_stream(ctx, &reader, &mut encoder, &sender, &cancelled).await
                }
            };
            if let Err(error) = result {
                let _ = sender.send_async(Err(error.to_string())).await;
            }
            drain_wait_until(ctx).await
        }
    }
}

async fn pump_response_stream<'js>(
    ctx: &Ctx<'js>,
    reader: &Object<'js>,
    encoder: &mut Option<ResponseEncoder>,
    sender: &Sender<BodyChunk>,
    cancelled: &CancellationToken,
) -> Result<(), Error> {
    let read: Function = property(reader, "read")?;
    let cancel: Function = property(reader, "cancel")?;
    loop {
        if cancelled.is_cancelled() {
            cancel_response_stream(ctx, &cancel).await;
            return Ok(());
        }
        let pending: Promise = read
            .call(())
            .map_err(|error| js_error("response stream read", error))?;
        let chunk: Option<TypedArray<u8>> = tokio::select! {
            chunk = finish_promise(ctx, &pending, "response stream read") => chunk?,
            () = cancelled.cancelled() => {
                cancel_response_stream(ctx, &cancel).await;
                return Ok(());
            }
        };
        let Some(chunk) = chunk else {
            return send_response_chunk(&[], true, encoder, sender).await;
        };
        let Some(bytes) = chunk.as_bytes() else {
            return Err(Error::Engine(
                "response stream chunk was detached".to_owned(),
            ));
        };
        if let Err(error) = send_response_chunk(bytes, false, encoder, sender).await {
            cancel_response_stream(ctx, &cancel).await;
            return Err(error);
        }
    }
}

async fn send_response_chunk(
    mut bytes: &[u8],
    finish: bool,
    encoder: &mut Option<ResponseEncoder>,
    sender: &Sender<BodyChunk>,
) -> Result<(), Error> {
    loop {
        let (consumed, output, drained) = match encoder {
            Some(encoder) => encoder.step(bytes, finish)?,
            None => (bytes.len(), bytes.to_vec(), true),
        };
        if !output.is_empty() {
            sender
                .send_async(Ok(output))
                .await
                .map_err(|_| io::Error::other("HTTP response receiver closed"))?;
        }
        bytes = &bytes[consumed..];
        if drained {
            return Ok(());
        }
    }
}

async fn cancel_response_stream<'js>(ctx: &Ctx<'js>, cancel: &Function<'js>) {
    let Ok(pending) = cancel.call::<_, Promise>(()) else {
        return;
    };
    let _ = finish_promise::<Value>(ctx, &pending, "response stream cancel").await;
}

async fn finish_promise<'js, T: rquickjs::FromJs<'js>>(
    ctx: &Ctx<'js>,
    promise: &Promise<'js>,
    stage: &'static str,
) -> Result<T, Error> {
    let _waiting = crate::event_loop::PromiseWait::enter(ctx, stage);
    promise
        .clone()
        .into_future::<T>()
        .await
        .map_err(|error| js_exception(ctx, stage, error))
}

/// Give the runtime the app's `assets`: their binding's name and `fetch`.
/// Bind the app's assets in the Worker's environment through the runtime's
/// `installAssets`.
fn install_assets<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    assets: &Arc<Assets>,
) -> rquickjs::Result<()> {
    let fetch_assets = Arc::clone(assets);
    let fetch = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, method: String, path: String| {
            asset_response(&ctx, fetch_assets.fetch(&method, &path))
        },
    )?;
    let install: Function = bootstrap.get("installAssets")?;
    install.call((assets.binding(), fetch))
}

/// `response` as the object the assets binding builds its `Response` from.
fn asset_response<'js>(ctx: &Ctx<'js>, response: AssetResponse) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("status", response.status.as_u16())?;
    object.set("statusText", response.status.canonical_reason())?;
    object.set("contentType", response.content_type)?;
    let body = response.body.map(|body| TypedArray::new(ctx.clone(), body));
    object.set("body", body.transpose()?)?;
    Ok(object)
}

/// The `name` property of `object`.
fn property<'js, T: rquickjs::FromJs<'js>>(object: &Object<'js>, name: &str) -> Result<T, Error> {
    object.get(name).map_err(|error| js_error(name, error))
}

/// The function `source` evaluates to.
fn eval_function<'js>(ctx: &Ctx<'js>, source: &str, stage: &str) -> Result<Function<'js>, Error> {
    ctx.eval(source).map_err(|error| js_error(stage, error))
}

fn js_error(stage: &str, error: impl std::fmt::Display) -> Error {
    Error::Engine(format!("{stage}: {error}"))
}

fn js_exception(ctx: &Ctx<'_>, stage: &str, error: impl std::fmt::Display) -> Error {
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
