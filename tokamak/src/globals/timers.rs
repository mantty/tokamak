use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use rquickjs::{Class, Ctx, Exception, Function, JsLifetime};
use tokamak_macros::host_function;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::cancellation::Cancellation;
use crate::typed::JsFunction;

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

/// The context's timers, in the order they are due.
#[derive(JsLifetime)]
struct Timers(Rc<TimerQueue>);

/// Runs `callback` after `milliseconds`, after timers due earlier.
#[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
#[host_function]
pub(super) fn schedule_timer<'js>(
    ctx: Ctx<'js>,
    callback: JsFunction<'js, ()>,
    milliseconds: u64,
) -> rquickjs::Result<Class<'js, Cancellation>> {
    let queue = timer_queue(&ctx)?;
    let cancelled = schedule(&ctx, &queue, callback.into_inner(), milliseconds);
    Cancellation::instance(&ctx, cancelled)
}

/// The context's timer queue, created when it schedules its first timer.
fn timer_queue(ctx: &Ctx<'_>) -> rquickjs::Result<Rc<TimerQueue>> {
    if let Some(timers) = ctx.userdata::<Timers>() {
        return Ok(Rc::clone(&timers.0));
    }
    let queue = Rc::new(TimerQueue::default());
    ctx.store_userdata(Timers(Rc::clone(&queue)))
        .map_err(|_| Exception::throw_internal(ctx, "The timer queue could not be stored"))?;
    Ok(queue)
}

/// Runs `callback` after `milliseconds`, unless the token it returns is
/// cancelled first.
fn schedule<'js>(
    ctx: &Ctx<'js>,
    queue: &Rc<TimerQueue>,
    callback: Function<'js>,
    milliseconds: u64,
) -> CancellationToken {
    let cancelled = CancellationToken::new();
    let cancelling = cancelled.clone();
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
            () = cancelled.cancelled() => {},
            () = async {
                tokio::time::sleep_until(deadline).await;
                // Tokio may wake simultaneous timers in any order. Only the
                // earliest live timer may run a JS callback and its microtasks.
                loop {
                    let changed = pending.queue.changed.notified();
                    if pending.queue.pending.borrow().first() == Some(&pending.key) { break; }
                    changed.await;
                }
            } => super::run_task(&callback, (), &cancelled),
        }
        drop(pending);
    });
    cancelling
}

#[cfg(test)]
mod tests {
    use rquickjs::{AsyncContext, AsyncRuntime, Function};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    struct Capture(Arc<AtomicUsize>);

    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// What a 0 ms timer that is due when first polled, and the microtask
    /// `microtask` queued before it, push to `order`. `clear` cancels the timer.
    fn order_with_a_due_timer(microtask: &str) -> Result<String, Box<dyn std::error::Error>> {
        let executor = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        // Another thread turns the time driver, so the timer is due when it is first polled.
        executor.spawn(async {
            loop {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        });
        let order = executor.block_on(due_timer_order(microtask))?;
        Ok(order)
    }

    async fn due_timer_order(microtask: &str) -> rquickjs::Result<String> {
        let runtime = AsyncRuntime::new()?;
        let context = AsyncContext::full(&runtime).await?;
        context
            .async_with(async |ctx| {
                ctx.eval::<(), _>("globalThis.order = []")?;
                let callback: Function = ctx.eval("() => order.push('timeout')")?;
                let queue = std::rc::Rc::new(super::TimerQueue::default());
                let cancelled = super::schedule(&ctx, &queue, callback, 0);
                let clear = Function::new(ctx.clone(), move || cancelled.cancel())?;
                ctx.globals().set("clear", clear)?;
                ctx.eval::<(), _>(format!("Promise.resolve().then(() => {{ {microtask} }})"))?;
                std::thread::sleep(Duration::from_millis(5));
                // The timer leaves the queue once it has run or been cleared.
                while !queue.pending.borrow().is_empty() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                ctx.eval::<String, _>("order.join()")
            })
            .await
    }

    #[test]
    fn runs_waiting_microtasks_before_a_timer_that_is_due() -> Result<(), Box<dyn std::error::Error>>
    {
        assert_eq!(
            order_with_a_due_timer("order.push('microtask')")?,
            "microtask,timeout"
        );
        Ok(())
    }

    #[test]
    fn skips_a_due_timer_that_a_waiting_microtask_clears() -> Result<(), Box<dyn std::error::Error>>
    {
        assert_eq!(
            order_with_a_due_timer("clear(); order.push('microtask')")?,
            "microtask"
        );
        Ok(())
    }

    #[test]
    fn releases_callbacks_when_timers_are_cancelled_or_the_request_ends()
    -> Result<(), Box<dyn std::error::Error>> {
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        for cancel in [false, true] {
            let drops = Arc::new(AtomicUsize::new(0));
            executor.block_on(schedule_hour_timer(&drops, cancel))?;
            assert_eq!(drops.load(Ordering::Relaxed), 1);
        }
        Ok(())
    }

    /// Schedules an hour-long timer whose callback counts its drop in `drops`,
    /// and checks that only clearing it, when `cancel` is set, releases the
    /// callback before the runtime ends.
    async fn schedule_hour_timer(
        drops: &Arc<AtomicUsize>,
        cancel: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let runtime = AsyncRuntime::new()?;
        let context = AsyncContext::full(&runtime).await?;
        context
            .async_with(async |ctx| {
                let capture = Capture(Arc::clone(drops));
                let callback = Function::new(ctx.clone(), move || {
                    std::hint::black_box(&capture);
                })?;
                let queue = std::rc::Rc::new(super::TimerQueue::default());
                let cancelled = super::schedule(&ctx, &queue, callback, 3_600_000);
                if cancel {
                    cancelled.cancel();
                }
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
        Ok(())
    }
}
