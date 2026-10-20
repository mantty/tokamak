//! The Worker's native calls on the shell's plugins.

use std::future::Future;
use std::sync::Arc;

use rquickjs::convert::List;
use rquickjs::function::Opt;
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, IntoJs, Object, Promise, TypedArray, Value};
use tokio_util::sync::CancellationToken;

use crate::plugin_calls::{Body, Overflow, PluginCalls, PluginReply, Subscription, error};

/// Gives the Worker its native calls and invocation-owned subscriptions.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    calls: &Arc<PluginCalls>,
) -> rquickjs::Result<()> {
    let calling = Arc::clone(calls);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              plugin: String,
              method: String,
              arguments: ArrayBuffer<'js>,
              pull: Opt<Function<'js>>| {
            let payload = arguments
                .as_bytes()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
            let cancelled = CancellationToken::new();
            let result = match pull.0 {
                None => settle(&ctx, calling.call(&plugin, &method, payload), &cancelled)?,
                Some(pull) => {
                    let (result, body) = calling.call_with_body(&plugin, &method, payload);
                    if let Some(body) = body {
                        read_body(&ctx, body, pull);
                    }
                    settle(&ctx, result, &cancelled)?
                }
            };
            let cancel = Function::new(ctx.clone(), move || cancelled.cancel())?;
            Ok::<_, rquickjs::Error>(List((result, cancel)))
        },
    )?;
    let subscribing = Arc::clone(calls);
    let subscribe = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              plugin: String,
              method: String,
              arguments: ArrayBuffer<'js>,
              deliver: Function<'js>,
              overflow: String| {
            let payload = arguments
                .as_bytes()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
            let overflow = Overflow::named(&overflow)
                .ok_or_else(|| Exception::throw_type(&ctx, "Unknown overflow policy"))?;
            let subscription = subscribing.subscribe(&plugin, &method, payload, overflow);
            listen(&ctx, subscription, deliver)
        },
    )?;
    bootstrap
        .get::<_, Function>("installPlugins")?
        .call((call, subscribe))
}

/// The promise of `result`, which rejects with `AbortError` once
/// `cancelled`, cancelling the native work.
fn settle<'js>(
    ctx: &Ctx<'js>,
    result: impl Future<Output = PluginReply> + 'static,
    cancelled: &CancellationToken,
) -> rquickjs::Result<Promise<'js>> {
    let context = ctx.clone();
    let cancelled = cancelled.clone();
    Promise::wrap_future(ctx, async move {
        let result = tokio::select! {
            result = result => result,
            () = cancelled.cancelled() => error("AbortError", "The native call was cancelled"),
        };
        reply(&context, result)
    })
}

/// Answers each of the shell's pulls on `body` with the chunk `pull`
/// resolves to, until the last or the end of the call.
fn read_body<'js>(ctx: &Ctx<'js>, mut body: Body, pull: Function<'js>) {
    ctx.spawn(async move {
        while body.pulled().await {
            let chunk = read_chunk(&pull)
                .await
                .unwrap_or_else(|_| error("DataCloneError", "The call's body could not be read"));
            body.send(&chunk);
            if chunk.done {
                return;
            }
        }
    });
}

async fn read_chunk(pull: &Function<'_>) -> rquickjs::Result<PluginReply> {
    let promise: Promise = pull.call(())?;
    let chunk: Object = promise.into_future().await?;
    let payload: TypedArray<u8> = chunk.get("payload")?;
    Ok(PluginReply {
        payload: payload.as_bytes().unwrap_or_default().to_vec(),
        failed: chunk.get("failed")?,
        done: chunk.get("done")?,
    })
}

/// Passes each of `subscription`'s results to `deliver` until the last, or
/// until the returned function is called. Discarding the function leaves the
/// listener registered.
fn listen<'js>(
    ctx: &Ctx<'js>,
    mut subscription: Subscription,
    deliver: Function<'js>,
) -> rquickjs::Result<Function<'js>> {
    let removed = CancellationToken::new();
    let remove = removed.clone();
    ctx.spawn(async move {
        loop {
            let result = tokio::select! {
                biased;
                () = removed.cancelled() => return,
                result = subscription.next() => result,
            };
            let Some(result) = result else { return };
            super::run_task(&deliver, (result,), &removed);
        }
    });
    Function::new(ctx.clone(), move || remove.cancel())
}

impl<'js> IntoJs<'js> for PluginReply {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        Ok(reply(ctx, self)?.into_value())
    }
}

fn reply<'js>(ctx: &Ctx<'js>, result: PluginReply) -> rquickjs::Result<Object<'js>> {
    let value = Object::new(ctx.clone())?;
    value.set("payload", ArrayBuffer::new(ctx.clone(), result.payload)?)?;
    value.set("failed", result.failed)?;
    value.set("done", result.done)?;
    Ok(value)
}
