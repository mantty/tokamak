use super::invocation::Invocation;
use super::{
    finish_promise, js_exception, load_worker_exports, prepare_worker_context, worker_runtime,
};
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::services::{Call, Lifetime, Reply, Scope};
use rquickjs::{Function, Object, Promise};
use std::sync::Arc;

pub(crate) async fn execute_service(
    worker: &WorkerBundle,
    config: &RuntimeConfig,
    call: Call,
    lifetime: &Lifetime,
    reply: &flume::Sender<Result<Reply, Error>>,
) -> Result<(), Error> {
    let services = Scope::within(lifetime);
    let owner = Invocation::new(lifetime.clone(), Arc::clone(&services.streams));
    let (runtime, context) = worker_runtime(worker, config, &owner).await?;
    let awaited = crate::event_loop::AwaitedPromise::default();
    let run = ServiceRun {
        worker,
        config,
        awaited: &awaited,
        services: &services,
        owner: &owner,
    };
    let invocation =
        context.async_with(async |ctx| -> Result<Reply, Error> { invoke(&ctx, &run, call).await });
    let result = tokio::select! {
        result = crate::event_loop::run(&runtime, &awaited, invocation) => result,
        () = lifetime.cancelled() => Err(Error::Engine("Service invocation was cancelled".into())),
    };
    let _ = reply.send(result);
    drain_streams(&runtime, &awaited, &context, &services, &owner, lifetime).await;
    if let Err(error) = owner
        .finish(&runtime, &context, &awaited, services.streams.until_none())
        .await
    {
        eprintln!("tokamak service waitUntil: {error}");
    }
    Ok(())
}

/// What a service invocation runs against.
struct ServiceRun<'a> {
    worker: &'a WorkerBundle,
    config: &'a RuntimeConfig,
    awaited: &'a crate::event_loop::AwaitedPromise,
    services: &'a Scope,
    owner: &'a Arc<Invocation>,
}

/// Runs `call` in the service Worker's fresh runtime.
async fn invoke(ctx: &rquickjs::Ctx<'_>, run: &ServiceRun<'_>, call: Call) -> Result<Reply, Error> {
    let bootstrap = prepare_worker_context(
        ctx,
        run.worker,
        run.config,
        run.awaited,
        run.services,
        run.owner,
    )?;
    let exports = load_worker_exports(ctx, run.worker).await?;
    let arguments = call
        .arguments
        .into_js(ctx, &bootstrap, &run.services.streams)
        .map_err(|error| js_exception(ctx, "service arguments", error))?;
    let invoke: Function = bootstrap
        .get("invokeService")
        .map_err(|error| js_exception(ctx, "service dispatcher", error))?;
    let pending: Promise = invoke
        .call((exports, call.entrypoint, call.method, arguments))
        .map_err(|error| js_exception(ctx, "service invocation", error))?;
    let result: Object = finish_promise(ctx, &pending, "service result").await?;
    Reply::from_js(&result, &bootstrap, &run.services.streams)
        .map_err(|error| js_exception(ctx, "service result", error))
}

/// Runs the service Worker's streams until they close or `lifetime` ends.
async fn drain_streams(
    runtime: &rquickjs::AsyncRuntime,
    awaited: &crate::event_loop::AwaitedPromise,
    context: &rquickjs::AsyncContext,
    services: &Scope,
    owner: &Arc<Invocation>,
    lifetime: &Lifetime,
) {
    let streaming = context.async_with(async |_ctx| {
        services.streams.until_none().await;
        Ok(())
    });
    tokio::select! {
        result = crate::event_loop::run(runtime, awaited, streaming) => {
            if let Err(error) = result { eprintln!("tokamak service stream: {error}"); }
        }
        () = lifetime.cancelled() => {
            owner.background_deadline();
            services.streams.cancel();
        },
    }
}
