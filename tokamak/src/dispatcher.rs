mod bootstrap;
mod invocation;
mod service;
pub(crate) use bootstrap::Bootstrap;
pub(crate) use service::execute_service;

use flume::Sender;
use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::assets::Assets;
use crate::compat;
use crate::fs::VirtualFileSystem;
use crate::fs::{
    MODULE_NAME as NODE_FS_MODULE_NAME, NodeFsModule, NodeFsPromisesModule,
    PROMISES_MODULE_NAME as NODE_FS_PROMISES_MODULE_NAME, install, read_bundle_file,
};
use crate::gateway::{
    Delivery, EventJob, Handler, HandlerError, Job, JobResponse, WebSocketInbound, WebSocketJob,
    WebSocketOutbound, WebSocketOutgoing,
};
use crate::globals::ResponseEncoder;
use crate::linked::StorageRuntime;
use crate::packaging::{self, ModuleType};
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::transport::{BodyChunk, HttpRequest, HttpResponse, append_header, response_stream};
use crate::typed::JsPromise;
use bootstrap::{
    AssetFetcher, BodyReader, NativeSocket, RequestDescriptor, ResponseBody, ResponseParts,
    SocketNotifier, WebSocketData,
};
use hyper::header::HeaderMap;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    ArrayBuffer, AsyncContext, AsyncRuntime, Class, Ctx, Function, Module, Object, Promise,
    TypedArray, Value,
};

/// Dispatches packaged application requests to the app's assets or, through
/// `QuickJS`, its Worker.
pub(crate) struct Dispatcher {
    worker: Arc<WorkerBundle>,
    config: RuntimeConfig,
    native: Option<Arc<Self>>,
}

impl Dispatcher {
    pub(crate) fn new(bundle: WorkerBundle, config: RuntimeConfig) -> Arc<Self> {
        Arc::new(Self {
            worker: Arc::new(bundle),
            config,
            native: None,
        })
    }

    pub(crate) fn with_native(
        bundle: WorkerBundle,
        config: RuntimeConfig,
        native: Arc<Self>,
    ) -> Arc<Self> {
        Arc::new(Self {
            worker: Arc::new(bundle),
            config,
            native: Some(native),
        })
    }
}

impl Handler for Dispatcher {
    fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError> {
        if let Some(native) = &self.native
            && job.request.target.starts_with(crate::NATIVE_ROUTE_PREFIX)
        {
            return execute_request(&native.worker, &native.config, job, stopped)
                .map_err(Into::into);
        }
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

    /// Delivers `event` to the Tokamak worker. Without one, nothing listens.
    fn deliver(&self, event: EventJob, stopped: &CancellationToken) {
        let Some(native) = &self.native else {
            let _ = event.delivered.send(Ok(Delivery {
                reply: Vec::new(),
                listened: Some(std::collections::BTreeSet::new()),
            }));
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(tokio) => tokio.block_on(execute_event(
                &native.worker,
                &native.config,
                event,
                stopped,
            )),
            Err(error) => {
                let _ = event.delivered.send(Err(error.into()));
            }
        }
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
/// interrupted once `owner` is.
async fn worker_runtime(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    owner: &Arc<invocation::Invocation>,
) -> Result<(AsyncRuntime, AsyncContext), Error> {
    let runtime = AsyncRuntime::new().map_err(|error| js_error("runtime", error))?;
    let interrupted = Arc::clone(owner);
    runtime
        .set_interrupt_handler(Some(Box::new(move || interrupted.interrupted())))
        .await;
    runtime
        .set_loader(
            WorkerResolver,
            WorkerLoader {
                bundle: worker.clone(),
                storage: config.storage.clone(),
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
    let lifetime = crate::services::Lifetime {
        caller: job.cancelled.clone(),
        stopped: stopped.clone(),
        deadline: job.deadline,
    };
    let services = crate::services::Scope::within(&lifetime);
    let owner = invocation::Invocation::new(lifetime.clone(), Arc::clone(&services.streams));
    let (runtime, context) = worker_runtime(worker, config, &owner).await?;
    let awaited = crate::event_loop::AwaitedPromise::default();
    let request = context.async_with(async |ctx| -> Result<(), Error> {
        let Job {
            cancelled: _,
            deadline: _,
            request,
            response: response_sender,
            websocket,
        } = job;
        let bootstrap = prepare_worker_context(&ctx, worker, config, &awaited, &services, &owner)?;
        install_request(&ctx, &request)?;
        let response = tokio::select! {
            response = invoke_worker(&ctx, worker, &bootstrap, &request) => response?,
            () = lifetime.cancelled() => return Ok(()),
        };
        let response = response_from_js(&ctx, &bootstrap, response)?;
        if response.status == 101
            && let (Some(native), Some(websocket)) = (&response.web_socket, websocket)
        {
            response_sender
                .send_async(JobResponse::WebSocket)
                .await
                .map_err(|_| receiver_closed("WebSocket response receiver closed"))?;
            websocket_loop(&ctx, native, &websocket, stopped).await?;
            return Ok(());
        }
        send_worker_response(&ctx, response, &response_sender, &owner).await
    });
    let request = crate::event_loop::run(&runtime, &awaited, request);
    let result = tokio::select! {
        result = request => result,
        () = lifetime.terminated() => Ok(()),
    };
    let background = owner
        .finish(&runtime, &context, &awaited, std::future::ready(()))
        .await;
    if owner.caller_interrupted() {
        Ok(())
    } else {
        result.and(background)
    }
}

/// Runs the Worker's listeners of `event` in a fresh `QuickJS` runtime, as a
/// request runs, interrupting them at `event.deadline`, and sends what they
/// did to `event.delivered`. The reply stands once they return, whether or not
/// their `waitUntil` promises settle before the deadline.
pub(super) async fn execute_event(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    event: EventJob,
    stopped: &CancellationToken,
) {
    let EventJob {
        name,
        event,
        deadline,
        delivered,
    } = event;
    let lifetime = crate::services::Lifetime {
        caller: stopped.child_token(),
        stopped: stopped.clone(),
        deadline: Some(deadline),
    };
    let services = crate::services::Scope::until(stopped, deadline);
    let owner = invocation::Invocation::new(lifetime, Arc::clone(&services.streams));
    let (runtime, context) = match worker_runtime(worker, config, &owner).await {
        Ok(invocation) => invocation,
        Err(error) => {
            let _ = delivered.send(Err(error.into()));
            return;
        }
    };
    let awaited = crate::event_loop::AwaitedPromise::default();
    let dispatch = context.async_with(async |ctx| -> Result<Delivery, Error> {
        let bootstrap = prepare_worker_context(&ctx, worker, config, &awaited, &services, &owner)?;
        let exports = load_worker_exports(&ctx, worker).await?;
        let event = TypedArray::new(ctx.clone(), event)
            .map_err(|error| js_error("event payload", error))?;
        let pending = bootstrap
            .dispatch_event
            .call((exports, name.clone(), event))
            .map_err(|error| js_exception(&ctx, &name, error))?;
        let dispatched = finish_promise(&ctx, pending, "event").await?;
        let bootstrap::DispatchedEvent { reply, listened } =
            rquickjs::FromJs::from_js(&ctx, dispatched)
                .map_err(|error| js_exception(&ctx, "event", error))?;
        Ok(Delivery {
            reply: reply
                .as_bytes()
                .ok_or_else(|| js_error("event", "detached reply buffer"))?
                .to_vec(),
            listened: listened.map(|names| names.into_iter().collect()),
        })
    });
    let deadline = tokio::time::Instant::from_std(deadline);
    let delivery = tokio::select! {
        delivery = crate::event_loop::run(&runtime, &awaited, dispatch) => delivery,
        () = tokio::time::sleep_until(deadline) => {
            let message = format!("{name} did not complete before its deadline");
            Err(io::Error::new(io::ErrorKind::TimedOut, message).into())
        }
        () = stopped.cancelled() => return,
    };
    if delivery.is_ok()
        && let Err(error) = owner
            .finish(&runtime, &context, &awaited, std::future::ready(()))
            .await
    {
        eprintln!("tokamak event waitUntil: {error}");
    }
    let _ = delivered.send(delivery.map_err(Into::into));
}

/// Installs the Worker's globals, the runtime bootstrap, the app's assets and
/// the native calls on its plugins, and gives `owner` the Worker's `waitUntil`
/// work, returning the bootstrap's exports.
fn prepare_worker_context<'js>(
    ctx: &Ctx<'js>,
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    awaited: &crate::event_loop::AwaitedPromise,
    services: &crate::services::Scope,
    owner: &Arc<invocation::Invocation>,
) -> Result<Bootstrap<'js>, Error> {
    ctx.store_userdata(awaited.clone())
        .map_err(|error| js_error("event loop", error))?;
    install_worker_globals(ctx, config)?;
    let bootstrap = initialize_worker_context(ctx, worker)?;
    if let Some(assets) = &config.assets {
        install_assets(ctx, &bootstrap, assets)
            .map_err(|error| js_exception(ctx, "assets", error))?;
    }
    if let Some(plugins) = &config.plugins {
        crate::globals::plugins::install(ctx, &bootstrap, plugins)
            .map_err(|error| js_exception(ctx, "plugins", error))?;
    }
    if let Some(bindings) = &config.services {
        crate::services::install(ctx, &bootstrap, bindings, services)
            .map_err(|error| js_exception(ctx, "service bindings", error))?;
    }
    owner.track_wait_until(ctx, &bootstrap)?;
    Ok(bootstrap)
}

async fn invoke_worker<'js>(
    ctx: &Ctx<'js>,
    worker: &WorkerBundle,
    bootstrap: &Bootstrap<'js>,
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
    let request = bootstrap
        .host_request
        .call((RequestDescriptor::new(request), body))
        .map_err(|error| js_exception(ctx, "request", error))?;
    let environment: Object = global(ctx, "__tokamak_env")?;
    let execution_context: Object = global(ctx, "__tokamak_context")?;
    let response: Promise = fetch
        .call((request, environment, execution_context))
        .map_err(|error| js_error("fetch", error))?;
    finish_promise(ctx, response, "response").await
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

/// Install the request's descriptor as `__tokamak_request`.
fn install_request(ctx: &Ctx<'_>, request: &HttpRequest) -> Result<(), Error> {
    ctx.globals()
        .set("__tokamak_request", RequestDescriptor::new(request))
        .map_err(|error| js_error("setup", error))
}

/// Install the Node file system and evaluate the runtime bootstrap, returning
/// its exports.
fn initialize_worker_context<'js>(
    ctx: &Ctx<'js>,
    worker: &WorkerBundle,
) -> Result<Bootstrap<'js>, Error> {
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
    let exports = load_worker_exports(ctx, bundle).await?;
    let execution_context: Value = global(ctx, "__tokamak_context")?;
    let environment: Value = global(ctx, "__tokamak_env")?;
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

/// Evaluate the Worker's entry module and return its exports.
async fn load_worker_exports<'js>(
    ctx: &Ctx<'js>,
    bundle: &WorkerBundle,
) -> Result<Object<'js>, Error> {
    let bytes = read_worker_module(bundle, &bundle.entry)?;
    let module = unsafe { Module::load(ctx.clone(), &bytes) }
        .map_err(|error| js_exception(ctx, "load", error))?;
    let (module, evaluation) = module.eval().map_err(|error| js_error("evaluate", error))?;
    finish_promise::<()>(ctx, evaluation, "module initialization").await?;
    let exports = module
        .namespace()
        .map_err(|error| js_error("worker exports", error))?;
    ctx.globals()
        .set("__tokamak_exports", exports.clone())
        .map_err(|error| js_error("worker exports", error))?;
    let execution_context: Value = global(ctx, "__tokamak_context")?;
    if let Some(context) = execution_context.as_object() {
        context
            .set("exports", exports.clone())
            .map_err(|error| js_error("execution context exports", error))?;
    }
    Ok(exports)
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
        if !is_module_name(&resolved)
            || (resolved.starts_with("tokamak:") && !base.starts_with("tokamak:"))
        {
            return Err(rquickjs::Error::new_resolving(base, name));
        }
        Ok(resolved)
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
    native: &NativeSocket<'js>,
    websocket: &WebSocketJob,
    stopped: &CancellationToken,
) -> Result<(), Error> {
    let changed = Arc::new(tokio::sync::Notify::new());
    let notifier = Class::instance(ctx.clone(), SocketNotifier::new(Arc::clone(&changed)))
        .map_err(|error| js_error("WebSocket notification", error))?;
    native
        .listen
        .call((notifier,))
        .map_err(|error| js_exception(ctx, "WebSocket", error))?;
    drain_pending_jobs(ctx);
    let mut open = true;
    while open {
        drain_websocket_outbox(native, &websocket.outgoing).await?;
        websocket
            .outgoing
            .send_async(WebSocketOutbound::Ready)
            .await;
        let event = loop {
            if stopped.is_cancelled() {
                return Ok(());
            }
            tokio::select! {
                event = websocket.incoming.recv_async() => break event,
                () = changed.notified() => {
                    drain_websocket_outbox(native, &websocket.outgoing).await?;
                },
                () = stopped.cancelled() => return Ok(()),
            }
        };
        open = deliver_websocket_event(ctx, native, event)?;
        drain_pending_jobs(ctx);
    }
    Ok(())
}

/// Delivers the host's WebSocket `event` to the runtime's `receive`, `close`
/// or `disconnect`, returning whether the connection is still open.
fn deliver_websocket_event<'js>(
    ctx: &Ctx<'js>,
    native: &NativeSocket<'js>,
    event: Result<WebSocketInbound, flume::RecvError>,
) -> Result<bool, Error> {
    match event {
        Ok(WebSocketInbound::Message { binary, payload }) => {
            native
                .receive
                .call((WebSocketData { binary, payload },))
                .map_err(|error| js_exception(ctx, "WebSocket message", error))?;
            Ok(true)
        }
        // The client's close frame ended the connection.
        Ok(WebSocketInbound::Close { code, reason }) => {
            native
                .close
                .call((code, reason))
                .map_err(|error| js_exception(ctx, "WebSocket close", error))?;
            Ok(false)
        }
        // The connection ended without a close frame.
        Err(_) => {
            native
                .disconnect
                .call(())
                .map_err(|error| js_exception(ctx, "WebSocket close", error))?;
            Ok(false)
        }
    }
}

fn drain_pending_jobs(ctx: &Ctx<'_>) {
    while ctx.execute_pending_job() {}
}

async fn drain_websocket_outbox(
    native: &NativeSocket<'_>,
    outgoing: &WebSocketOutgoing,
) -> Result<(), Error> {
    let frames = native
        .take
        .call(())
        .map_err(|error| js_error("WebSocket outbox", error))?;
    for frame in frames {
        outgoing.send_async(frame.0).await;
    }
    Ok(())
}

fn receiver_closed(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, message)
}

async fn drain_wait_until(ctx: &Ctx<'_>) -> Result<(), Error> {
    let drain: Function = global(ctx, "__tokamak_drain_wait_until")?;
    let pending: Promise = drain
        .call(())
        .map_err(|error| js_error("waitUntil", error))?;
    finish_promise(ctx, pending, "waitUntil").await
}

struct JsResponse<'js> {
    status: u16,
    status_text: String,
    headers: HeaderMap,
    encoder: Option<ResponseEncoder>,
    body: JsResponseBody<'js>,
    /// The runtime's end of the response's WebSocket.
    web_socket: Option<NativeSocket<'js>>,
}

enum JsResponseBody<'js> {
    Buffered(Vec<u8>),
    Stream(BodyReader<'js>),
}

/// The parts of the Worker's `response` the runtime's `hostResponse` gives the host.
fn response_from_js<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Bootstrap<'js>,
    response: Value<'js>,
) -> Result<JsResponse<'js>, Error> {
    let ResponseParts {
        status,
        status_text,
        headers: entries,
        encode_body,
        body,
        web_socket,
    } = bootstrap
        .host_response
        .call((response,))
        .map_err(|error| js_exception(ctx, "response", error))?;
    let mut headers = HeaderMap::new();
    for (name, value) in entries.0 {
        append_header(&mut headers, &name, &value)?;
    }
    let encoder = if encode_body == "manual" {
        None
    } else {
        headers
            .get("content-encoding")
            .and_then(|value| value.to_str().ok())
            .map(ResponseEncoder::new)
            .transpose()?
            .flatten()
    };
    let body = match body {
        ResponseBody::Buffered(bytes) => JsResponseBody::Buffered(
            bytes
                .as_bytes()
                .ok_or_else(|| Error::Engine("response body was detached".to_owned()))?
                .to_vec(),
        ),
        ResponseBody::Stream(reader) => JsResponseBody::Stream(reader),
    };
    Ok(JsResponse {
        status,
        status_text,
        headers,
        encoder,
        body,
        web_socket,
    })
}

async fn send_worker_response<'js>(
    ctx: &Ctx<'js>,
    response: JsResponse<'js>,
    response_sender: &Sender<JobResponse>,
    owner: &invocation::Invocation,
) -> Result<(), Error> {
    let JsResponse {
        status,
        status_text,
        headers,
        mut encoder,
        body,
        ..
    } = response;
    if encoder.is_none()
        && let JsResponseBody::Buffered(body) = body
    {
        let mut response = HttpResponse::buffered(status, headers, body);
        response.status_text = status_text;
        let _ = response_sender
            .send_async(JobResponse::Http(response))
            .await;
        return Ok(());
    }
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
            let result =
                pump_response_stream(ctx, &reader, &mut encoder, &sender, &cancelled).await;
            if cancelled.is_cancelled() || result.is_err() {
                owner.background_deadline();
                cancel_response_stream(ctx, &reader)?;
            }
            result
        }
    };
    if let Err(error) = result {
        let _ = sender.send_async(Err(error.to_string())).await;
    }
    Ok(())
}

async fn pump_response_stream<'js>(
    ctx: &Ctx<'js>,
    reader: &BodyReader<'js>,
    encoder: &mut Option<ResponseEncoder>,
    sender: &Sender<BodyChunk>,
    cancelled: &CancellationToken,
) -> Result<(), Error> {
    loop {
        if cancelled.is_cancelled() {
            return Ok(());
        }
        let pending = reader
            .read
            .call(())
            .map_err(|error| js_error("response stream read", error))?;
        let chunk = tokio::select! {
            chunk = finish_promise(ctx, pending, "response stream read") => chunk?,
            () = cancelled.cancelled() => {
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
        send_response_chunk(bytes, false, encoder, sender).await?;
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

fn cancel_response_stream<'js>(ctx: &Ctx<'js>, reader: &BodyReader<'js>) -> Result<(), Error> {
    let pending = reader
        .cancel
        .call(())
        .map_err(|error| js_exception(ctx, "response stream cancel", error))?
        .into_inner();
    let context: Object = global(ctx, "__tokamak_context")?;
    property::<Function>(&context, "waitUntil")?
        .call((pending,))
        .map_err(|error| js_exception(ctx, "response stream cleanup", error))
}

async fn finish_promise<'js, T: rquickjs::FromJs<'js>>(
    ctx: &Ctx<'js>,
    promise: impl Into<JsPromise<'js, T>>,
    stage: &'static str,
) -> Result<T, Error> {
    let _waiting = crate::event_loop::PromiseWait::enter(ctx, stage);
    promise
        .into()
        .into_future()
        .await
        .map_err(|error| js_exception(ctx, stage, error))
}

/// Bind the app's `assets` in the Worker's environment through the runtime's
/// `installAssets`.
fn install_assets<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Bootstrap<'js>,
    assets: &Arc<Assets>,
) -> rquickjs::Result<()> {
    let fetcher = Class::instance(ctx.clone(), AssetFetcher::new(Arc::clone(assets)))?;
    bootstrap
        .install_assets
        .call((assets.binding().to_owned(), fetcher))
}

/// The `name` property of `object`.
fn property<'js, T: rquickjs::FromJs<'js>>(object: &Object<'js>, name: &str) -> Result<T, Error> {
    object.get(name).map_err(|error| js_error(name, error))
}

/// The global `name` of `ctx`.
fn global<'js, T: rquickjs::FromJs<'js>>(ctx: &Ctx<'js>, name: &str) -> Result<T, Error> {
    ctx.globals()
        .get(name)
        .map_err(|error| js_error(name, error))
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
