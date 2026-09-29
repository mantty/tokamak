//! Native functions the storage bindings' JavaScript calls. Storage work runs
//! on the blocking thread pool, so it never blocks JavaScript.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rquickjs::function::Async;
use rquickjs::module::Exports;
use rquickjs::{Ctx, Exception, Function, Object, TypedArray};

use super::Storage;
use super::kv::Entry;

/// A packaged app's storage, installed into each request's context.
pub(crate) struct StorageHandle(pub(crate) Arc<Storage>);

// This userdata contains no JavaScript values, so changing the marker lifetime
// cannot change its representation or validity.
#[allow(clippy::elidable_lifetime_names)]
unsafe impl<'js> rquickjs::JsLifetime<'js> for StorageHandle {
    type Changed<'to> = StorageHandle;
}

/// Names `export_host_functions` exports from `tokamak:host`.
pub(crate) const HOST_EXPORTS: &[&str] = &[
    "d1Query",
    "kvGet",
    "kvGetMany",
    "kvPut",
    "kvDelete",
    "kvList",
];

pub(crate) fn export_host_functions<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
) -> rquickjs::Result<()> {
    exports.export("d1Query", Function::new(ctx.clone(), Async(d1_query))?)?;
    exports.export("kvGet", Function::new(ctx.clone(), Async(kv_get))?)?;
    exports.export("kvGetMany", Function::new(ctx.clone(), Async(kv_get_many))?)?;
    exports.export("kvPut", Function::new(ctx.clone(), Async(kv_put))?)?;
    exports.export("kvDelete", Function::new(ctx.clone(), Async(kv_delete))?)?;
    exports.export("kvList", Function::new(ctx.clone(), Async(kv_list))?)?;
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
        blocking(&ctx, move || Ok(storage.d1_query(&binding, &body, &format))).await?;
    let response = Object::new(ctx)?;
    response.set("body", body)?;
    response.set("bookmark", bookmark)?;
    Ok(response)
}

/// Resolve to `{ value, metadata }` for an unexpired key, or null.
async fn kv_get(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
) -> rquickjs::Result<Option<Object<'_>>> {
    let storage = storage(&ctx)?;
    let entry = blocking(&ctx, move || storage.kv(&binding)?.get(&key, now())).await?;
    entry.map(|entry| entry_object(&ctx, entry)).transpose()
}

/// Resolve to the entry, or null, for each key.
async fn kv_get_many(
    ctx: Ctx<'_>,
    binding: String,
    keys: Vec<String>,
) -> rquickjs::Result<Vec<Option<Object<'_>>>> {
    let storage = storage(&ctx)?;
    let entries = blocking(&ctx, move || storage.kv(&binding)?.get_many(&keys, now())).await?;
    entries
        .into_iter()
        .map(|entry| entry.map(|entry| entry_object(&ctx, entry)).transpose())
        .collect()
}

/// Store `value` under `key` with an expiration in seconds and JSON metadata.
async fn kv_put<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    value: TypedArray<'js, u8>,
    expiration: Option<i64>,
    metadata: Option<String>,
) -> rquickjs::Result<()> {
    let storage = storage(&ctx)?;
    let value = value
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
        .to_vec();
    blocking(&ctx, move || {
        storage
            .kv(&binding)?
            .put(&key, &value, expiration, metadata.as_deref(), now())
    })
    .await
}

async fn kv_delete(ctx: Ctx<'_>, binding: String, key: String) -> rquickjs::Result<()> {
    let storage = storage(&ctx)?;
    blocking(&ctx, move || storage.kv(&binding)?.delete(&key, now())).await
}

/// Resolve to a page of keys as JSON: `{ keys, cursor? }`.
async fn kv_list(
    ctx: Ctx<'_>,
    binding: String,
    prefix: String,
    cursor: String,
    limit: usize,
) -> rquickjs::Result<String> {
    let storage = storage(&ctx)?;
    blocking(&ctx, move || {
        let page = storage.kv(&binding)?.list(&prefix, &cursor, limit, now())?;
        serde_json::to_string(&page).map_err(|error| error.to_string())
    })
    .await
}

fn entry_object<'js>(ctx: &Ctx<'js>, entry: Entry) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("value", TypedArray::new(ctx.clone(), entry.value)?)?;
    object.set("metadata", entry.metadata)?;
    Ok(object)
}

/// Run `work` on the blocking thread pool, rejecting with its error.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx<'_>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> rquickjs::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result)
        .map_err(|error| Exception::throw_message(ctx, &error))
}

/// Seconds since the Unix epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn storage(ctx: &Ctx<'_>) -> rquickjs::Result<Arc<Storage>> {
    ctx.userdata::<StorageHandle>()
        .map(|handle| Arc::clone(&handle.0))
        .ok_or_else(|| Exception::throw_internal(ctx, "storage is not available"))
}
