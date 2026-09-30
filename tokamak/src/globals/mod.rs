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
pub(crate) use compression::http::ResponseEncoder;
mod crypto;
mod html_rewriter;
mod intl;
pub(crate) mod native;
mod objects;
mod timers;
mod url;

/// The value unless the engine produced an exception.
pub(super) fn checked(value: rquickjs::Value<'_>) -> rquickjs::Result<rquickjs::Value<'_>> {
    if value.is_exception() {
        Err(rquickjs::Error::Exception)
    } else {
        Ok(value)
    }
}
