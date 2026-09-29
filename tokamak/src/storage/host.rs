//! Native functions the storage bindings' JavaScript calls. Storage work runs
//! on the blocking thread pool, so it never blocks JavaScript.

use std::sync::Arc;

use rquickjs::function::Async;
use rquickjs::module::Exports;
use rquickjs::{Ctx, Exception, Function, Object};

use super::Storage;

/// A packaged app's storage, installed into each request's context.
pub(crate) struct StorageHandle(pub(crate) Arc<Storage>);

// This userdata contains no JavaScript values, so changing the marker lifetime
// cannot change its representation or validity.
#[allow(clippy::elidable_lifetime_names)]
unsafe impl<'js> rquickjs::JsLifetime<'js> for StorageHandle {
    type Changed<'to> = StorageHandle;
}

/// Names `export_host_functions` exports from `tokamak:host`.
pub(crate) const HOST_EXPORTS: &[&str] = &["d1Query"];

pub(crate) fn export_host_functions<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
) -> rquickjs::Result<()> {
    exports.export("d1Query", Function::new(ctx.clone(), Async(d1_query))?)?;
    Ok(())
}

/// Resolve to `{ body, bookmark }`: a D1 service response for `binding`.
async fn d1_query(
    ctx: Ctx<'_>,
    binding: String,
    format: String,
    body: String,
) -> rquickjs::Result<Object<'_>> {
    let storage = storage(&ctx)?;
    let (body, bookmark) =
        tokio::task::spawn_blocking(move || storage.d1_query(&binding, &body, &format))
            .await
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    let response = Object::new(ctx)?;
    response.set("body", body)?;
    response.set("bookmark", bookmark)?;
    Ok(response)
}

fn storage(ctx: &Ctx<'_>) -> rquickjs::Result<Arc<Storage>> {
    ctx.userdata::<StorageHandle>()
        .map(|handle| Arc::clone(&handle.0))
        .ok_or_else(|| Exception::throw_internal(ctx, "storage is not available"))
}
