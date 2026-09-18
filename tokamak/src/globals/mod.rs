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
