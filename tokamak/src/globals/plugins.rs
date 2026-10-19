//! The Worker's native calls on the shell's plugins.

use std::sync::Arc;

use rquickjs::{Ctx, Function, Object, Promise};
use tokio_util::sync::CancellationToken;

use crate::plugin_calls::{PluginCalls, Subscription};

/// Gives the Worker its native calls and invocation-owned subscriptions.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    calls: &Arc<PluginCalls>,
) -> rquickjs::Result<()> {
    let calling = Arc::clone(calls);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, plugin: String, method: String, arguments: String| {
            Promise::wrap_future(&ctx, calling.call(&plugin, &method, &arguments))
        },
    )?;
    let subscribing = Arc::clone(calls);
    let subscribe = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              plugin: String,
              method: String,
              arguments: String,
              deliver: Function<'js>| {
            let subscription = subscribing.subscribe(&plugin, &method, &arguments);
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
