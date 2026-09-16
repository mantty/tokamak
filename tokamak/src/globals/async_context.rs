use rquickjs::{Ctx, Value, qjs};

unsafe extern "C" {
    fn JS_GetAsyncContext(ctx: *mut qjs::JSContext) -> qjs::JSValue;
    fn JS_SetAsyncContext(ctx: *mut qjs::JSContext, value: qjs::JSValue);
}

pub(super) fn get(ctx: Ctx<'_>) -> Value<'_> {
    // The engine returns an owned reference belonging to this context.
    let value = unsafe { JS_GetAsyncContext(ctx.as_raw().as_ptr()) };
    unsafe { Value::from_raw(ctx, value) }
}

#[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
pub(super) fn set(value: Value<'_>) {
    // The engine retains its own reference; the argument remains borrowed.
    unsafe { JS_SetAsyncContext(value.ctx().as_raw().as_ptr(), value.as_raw()) };
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Function, Runtime};

    #[test]
    fn releases_abandoned_continuation_contexts() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> rquickjs::Result<()> {
            ctx.globals()
                .set("setContext", Function::new(ctx.clone(), super::set)?)?;
            ctx.eval::<(), _>(
                r"
                globalThis.makeCycles = () => {
                    for (let index = 0; index < 1000; index++) {
                        const frame = {};
                        setContext(frame);
                        const pending = new Promise(() => {});
                        frame.pending = pending;
                        pending.then(() => frame);
                    }
                    setContext(undefined);
                };
            ",
            )?;
            Ok(())
        })?;
        runtime.run_gc();
        let baseline = runtime.memory_usage().obj_count;
        for _ in 0..5 {
            context.with(|ctx| ctx.eval::<(), _>("makeCycles()"))?;
            runtime.run_gc();
            assert!(runtime.memory_usage().obj_count <= baseline + 2);
        }
        Ok(())
    }

    #[test]
    fn releases_queued_contexts_when_runtime_is_dropped() -> Result<(), Box<dyn std::error::Error>>
    {
        for _ in 0..10 {
            let runtime = Runtime::new()?;
            let context = Context::full(&runtime)?;
            context.with(|ctx| -> rquickjs::Result<()> {
                ctx.globals()
                    .set("setContext", Function::new(ctx.clone(), super::set)?)?;
                ctx.eval::<(), _>("setContext({}); queueMicrotask(() => {}); setContext(undefined)")
            })?;
        }
        Ok(())
    }
}
