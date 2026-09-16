use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use rquickjs::{Ctx, Function, function::MutFn};
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;

#[derive(Default)]
struct TimerQueue {
    next: Cell<u64>,
    pending: RefCell<BTreeSet<(Instant, u64)>>,
    changed: Notify,
}

struct PendingTimer {
    queue: Rc<TimerQueue>,
    key: (Instant, u64),
}

impl Drop for PendingTimer {
    fn drop(&mut self) {
        self.queue.pending.borrow_mut().remove(&self.key);
        self.queue.changed.notify_waiters();
    }
}

pub(super) fn scheduler(ctx: Ctx<'_>) -> rquickjs::Result<Function<'_>> {
    let queue = Rc::new(TimerQueue::default());
    Function::new(ctx, move |ctx, callback, milliseconds| {
        schedule(ctx, &queue, callback, milliseconds)
    })
}

#[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
fn schedule<'js>(
    ctx: Ctx<'js>,
    queue: &Rc<TimerQueue>,
    callback: Function<'js>,
    milliseconds: u64,
) -> rquickjs::Result<Function<'js>> {
    let (sender, cancelled) = oneshot::channel::<()>();
    let mut sender = Some(sender);
    let cancel = Function::new(
        ctx.clone(),
        MutFn::new(move || {
            sender.take();
        }),
    )?;
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    let sequence = queue.next.get();
    queue.next.set(sequence + 1);
    let pending = PendingTimer {
        queue: Rc::clone(queue),
        key: (deadline, sequence),
    };
    queue.pending.borrow_mut().insert(pending.key);
    ctx.spawn(async move {
        tokio::select! {
            biased;
            _ = cancelled => {},
            () = async {
                tokio::time::sleep_until(deadline).await;
                // Tokio may wake simultaneous timers in any order. Only the
                // earliest live timer may run a JS callback and its microtasks.
                loop {
                    let changed = pending.queue.changed.notified();
                    if pending.queue.pending.borrow().first() == Some(&pending.key) { break; }
                    changed.await;
                }
            } => {
                if callback.call::<_, ()>(()).is_err() {
                    let ctx = callback.ctx();
                    let error = ctx.catch();
                    if let Ok(report) = ctx.globals().get::<_, Function>("reportError") {
                        let _ = report.call::<_, ()>((error,));
                    }
                }
                while callback.ctx().execute_pending_job() {}
            }
        }
        drop(pending);
    });
    Ok(cancel)
}

#[cfg(test)]
mod tests {
    use rquickjs::{AsyncContext, AsyncRuntime, Function};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Capture(Arc<AtomicUsize>);

    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn releases_callbacks_when_timers_are_cancelled_or_the_request_ends()
    -> Result<(), Box<dyn std::error::Error>> {
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        for cancel in [false, true] {
            let drops = Arc::new(AtomicUsize::new(0));
            executor.block_on(async {
                let runtime = AsyncRuntime::new()?;
                let context = AsyncContext::full(&runtime).await?;
                context
                    .async_with(async |ctx| {
                        let capture = Capture(Arc::clone(&drops));
                        let callback = Function::new(ctx.clone(), move || {
                            std::hint::black_box(&capture);
                        })?;
                        let queue = std::rc::Rc::new(super::TimerQueue::default());
                        let clear = super::schedule(ctx.clone(), &queue, callback, 3_600_000)?;
                        if cancel {
                            clear.call::<_, ()>(())?;
                        }
                        ctx.globals().set("clear", clear)?;
                        Ok::<_, rquickjs::Error>(())
                    })
                    .await?;
                assert_eq!(drops.load(Ordering::Relaxed), 0);
                if cancel {
                    runtime
                        .execute_pending_job()
                        .await
                        .map_err(|_| "timer cancellation job failed")?;
                    assert_eq!(drops.load(Ordering::Relaxed), 1);
                }
                Ok::<_, Box<dyn std::error::Error>>(())
            })?;
            assert_eq!(drops.load(Ordering::Relaxed), 1);
        }
        Ok(())
    }
}
