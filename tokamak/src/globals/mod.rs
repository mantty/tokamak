/// Define `HOST_EXPORTS`, the names a host module declares, and
/// `export_host_functions`, which exports each name's native function.
macro_rules! host_functions {
    ($visibility:vis, $($name:literal => $function:expr),+ $(,)?) => {
        $visibility const HOST_EXPORTS: &[&str] = &[$($name),+];

        $visibility fn export_host_functions<'js>(
            ctx: &rquickjs::Ctx<'js>,
            exports: &rquickjs::module::Exports<'js>,
        ) -> rquickjs::Result<()> {
            $(exports.export($name, rquickjs::Function::new(ctx.clone(), $function)?)?;)+
            Ok(())
        }
    };
}
pub(crate) use host_functions;

mod async_context;
mod buffers;
pub(crate) mod compression;
pub(crate) use compression::http::{ContentDecoder, ResponseEncoder};
mod crypto;
mod html_rewriter;
mod intl;
pub(crate) mod native;
mod objects;
pub(crate) mod plugins;
mod timers;
mod url;

/// Runs `callback` with `args` as a task: first the microtasks waiting before
/// it, which rquickjs runs only after polling spawned futures such as this
/// task's, then, unless they cancelled it, the callback and the microtasks it
/// queues. Callbacks report their own errors; this clears any that escape.
fn run_task<'js>(
    callback: &rquickjs::Function<'js>,
    args: impl rquickjs::function::IntoArgs<'js>,
    cancelled: &tokio_util::sync::CancellationToken,
) {
    let ctx = callback.ctx();
    while ctx.execute_pending_job() {}
    if cancelled.is_cancelled() {
        return;
    }
    if callback.call::<_, ()>(args).is_err() {
        let _ = ctx.catch();
    }
    while ctx.execute_pending_job() {}
}

/// The value unless the engine produced an exception.
pub(super) fn checked(value: rquickjs::Value<'_>) -> rquickjs::Result<rquickjs::Value<'_>> {
    if value.is_exception() {
        Err(rquickjs::Error::Exception)
    } else {
        Ok(value)
    }
}
