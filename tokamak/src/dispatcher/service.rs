use super::invocation::Invocation;
use super::{
    finish_promise, js_exception, load_worker_exports, prepare_worker_context, worker_runtime,
};
use crate::event_loop::AwaitedPromise;
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::services::{Call, Lifetime, Reply, Scope};
use rquickjs::{AsyncContext, AsyncRuntime, Function, Object, Promise};
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
    let awaited = AwaitedPromise::default();
    let invocation = context.async_with(async |ctx| -> Result<Reply, Error> {
        let bootstrap = prepare_worker_context(&ctx, worker, config, &awaited, &services, &owner)?;
        let exports = load_worker_exports(&ctx, worker).await?;
        let arguments = call
            .arguments
            .into_js(&ctx, &bootstrap, &services.streams)
            .map_err(|error| js_exception(&ctx, "service arguments", error))?;
        let invoke: Function = bootstrap
            .get("invokeService")
            .map_err(|error| js_exception(&ctx, "service dispatcher", error))?;
        let pending: Promise = invoke
            .call((exports, call.entrypoint, call.method, arguments))
            .map_err(|error| js_exception(&ctx, "service invocation", error))?;
        let result: Object = finish_promise(&ctx, &pending, "service result").await?;
        Reply::from_js(&result, &bootstrap, &services.streams)
            .map_err(|error| js_exception(&ctx, "service result", error))
    });
    let result = tokio::select! {
        result = crate::event_loop::run(&runtime, &awaited, invocation) => result,
        () = lifetime.cancelled() => Err(Error::Engine("Service invocation was cancelled".into())),
    };
    let _ = reply.send(result);
    drain_streams(&runtime, &context, &awaited, &services, &owner, lifetime).await;
    if let Err(error) = owner
        .finish(&runtime, &context, &awaited, services.streams.until_none())
        .await
    {
        eprintln!("tokamak service waitUntil: {error}");
    }
    Ok(())
}

/// Runs the service Worker's streams until they close, or cancels them when
/// `lifetime` ends.
async fn drain_streams(
    runtime: &AsyncRuntime,
    context: &AsyncContext,
    awaited: &AwaitedPromise,
    services: &Scope,
    owner: &Invocation,
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
