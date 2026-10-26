use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Function, Object};

use super::{drain_wait_until, js_exception};
use crate::event_loop::AwaitedPromise;
use crate::quickjs::Error;
use crate::services::{Lifetime, Streams};

/// An invocation's registered work and execution deadline.
pub(super) struct Invocation {
    pub(super) lifetime: Lifetime,
    retained: AtomicUsize,
    background: OnceLock<Instant>,
    caller_interrupted: AtomicBool,
    streams: Arc<Streams>,
}

impl Invocation {
    pub(super) fn new(lifetime: Lifetime, streams: Arc<Streams>) -> Arc<Self> {
        Arc::new(Self {
            lifetime,
            retained: AtomicUsize::new(0),
            background: OnceLock::new(),
            caller_interrupted: AtomicBool::new(false),
            streams,
        })
    }

    pub(super) fn interrupted(&self) -> bool {
        if self.lifetime.expired() {
            return true;
        }
        if let Some(deadline) = self.background.get() {
            return Instant::now() >= *deadline;
        }
        if self.lifetime.caller.is_cancelled() {
            if self.retained.load(Ordering::Acquire) == 0 && !self.streams.active() {
                self.caller_interrupted.store(true, Ordering::Release);
                return true;
            }
            return self.background_deadline() <= Instant::now();
        }
        self.background
            .get()
            .is_some_and(|deadline| Instant::now() >= *deadline)
    }

    pub(super) fn caller_interrupted(&self) -> bool {
        self.caller_interrupted.load(Ordering::Acquire)
    }

    pub(super) fn background_deadline(&self) -> Instant {
        *self
            .background
            .get_or_init(|| self.lifetime.background_deadline())
    }

    pub(super) fn track_wait_until<'js>(
        self: &Arc<Self>,
        ctx: &Ctx<'js>,
        bootstrap: &Object<'js>,
    ) -> Result<(), Error> {
        let invocation = Arc::clone(self);
        let observe = Function::new(ctx.clone(), move |pending: bool| {
            if pending {
                invocation.retained.fetch_add(1, Ordering::AcqRel);
            } else {
                invocation.retained.fetch_sub(1, Ordering::AcqRel);
            }
        })
        .map_err(|error| js_exception(ctx, "waitUntil observer", error))?;
        bootstrap
            .get::<_, Function>("trackWaitUntil")
            .and_then(|track| track.call::<_, ()>((observe,)))
            .map_err(|error| js_exception(ctx, "waitUntil observer", error))
    }

    pub(super) async fn finish(
        &self,
        runtime: &AsyncRuntime,
        context: &AsyncContext,
        awaited: &AwaitedPromise,
        cleanup: impl Future<Output = ()> + Send,
    ) -> Result<(), Error> {
        let deadline = self.background_deadline();
        let drain = context.async_with(async |ctx| {
            cleanup.await;
            drain_wait_until(&ctx).await
        });
        tokio::select! {
            result = crate::event_loop::run(runtime, awaited, drain) => result,
            () = tokio::time::sleep_until(deadline.into()) => Err(Error::Engine("Worker waitUntil deadline exceeded".into())),
            () = self.lifetime.stopped.cancelled() => Ok(()),
        }
    }
}
