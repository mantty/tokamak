//! The Worker's native calls on the shell's plugins.

use std::sync::Arc;

use rquickjs::{ArrayBuffer, Ctx, Exception, Function, IntoJs, Object, Promise, Value};
use tokio_util::sync::CancellationToken;

use crate::plugin_calls::{PluginCalls, PluginReply, Subscription};

/// Gives the Worker its native calls and invocation-owned subscriptions.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    calls: &Arc<PluginCalls>,
) -> rquickjs::Result<()> {
    let calling = Arc::clone(calls);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, plugin: String, method: String, arguments: ArrayBuffer<'js>| {
            let payload = arguments
                .as_bytes()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
            let result = calling.call(&plugin, &method, payload);
            let context = ctx.clone();
            Promise::wrap_future(&ctx, async move { reply(&context, result.await) })
        },
    )?;
    let subscribing = Arc::clone(calls);
    let subscribe = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              plugin: String,
              method: String,
              arguments: ArrayBuffer<'js>,
              deliver: Function<'js>| {
            let payload = arguments
                .as_bytes()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
            let subscription = subscribing.subscribe(&plugin, &method, payload);
            listen(&ctx, subscription, deliver)
        },
    )?;
    bootstrap
        .get::<_, Function>("installPlugins")?
        .call((call, subscribe))
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
