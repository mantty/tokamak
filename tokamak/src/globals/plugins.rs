//! The Worker's native calls on the shell's plugins.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rquickjs::{Ctx, Function, Object, Promise};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::plugin_calls::{PluginCalls, Subscription};

/// The listeners a Worker invocation has registered, which keep it running.
#[derive(Default)]
pub(crate) struct Listeners {
    count: AtomicUsize,
    removed: Notify,
}

/// A registered listener, removed when dropped.
struct Listener(Arc<Listeners>);

impl Listeners {
    pub(crate) fn any(&self) -> bool {
        self.count.load(Ordering::Acquire) > 0
    }

    /// Waits until no listener is registered.
    pub(crate) async fn until_none(&self) {
        loop {
            let removed = self.removed.notified();
            if !self.any() {
                return;
            }
            removed.await;
        }
    }

    fn add(self: &Arc<Self>) -> Listener {
        self.count.fetch_add(1, Ordering::AcqRel);
        Listener(Arc::clone(self))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if self.0.count.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.removed.notify_waiters();
        }
    }
}

/// Gives the Worker in `ctx` its native calls on `calls`, through the
/// runtime bootstrap's `installPlugins`, counting its listeners in
/// `listeners`.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    calls: &Arc<PluginCalls>,
    listeners: &Arc<Listeners>,
) -> rquickjs::Result<()> {
    let calling = Arc::clone(calls);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, plugin: String, method: String, arguments: String| {
            Promise::wrap_future(&ctx, calling.call(&plugin, &method, &arguments))
        },
    )?;
    let subscribing = Arc::clone(calls);
    let listeners = Arc::clone(listeners);
    let subscribe = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              plugin: String,
              method: String,
              arguments: String,
              deliver: Function<'js>| {
            let subscription = subscribing.subscribe(&plugin, &method, &arguments);
            listen(&ctx, subscription, listeners.add(), deliver)
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
    listener: Listener,
    deliver: Function<'js>,
) -> rquickjs::Result<Function<'js>> {
    let removed = CancellationToken::new();
    let remove = removed.clone();
    ctx.spawn(async move {
        let _listener = listener;
        loop {
            let result = tokio::select! {
                biased;
                () = removed.cancelled() => return,
                result = subscription.next() => result,
            };
            let Some(result) = result else { return };
            // `deliver` reports what its callbacks throw; this clears anything that escapes.
            if deliver.call::<_, ()>((result,)).is_err() {
                let _ = deliver.ctx().catch();
            }
            while deliver.ctx().execute_pending_job() {}
        }
    });
    Function::new(ctx.clone(), move || remove.cancel())
}
