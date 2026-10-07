//! The `tokamak:storage` module: native functions the storage bindings'
//! JavaScript calls. Storage work runs on the blocking thread pool, so it
//! never blocks JavaScript.

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use super::kv::Entry;
use super::r2::{BodyReader, BodyWriter, Failure, R2Bucket, Read};
use super::{Storage, lock, text};
use rquickjs::class::Trace;
use rquickjs::function::Async;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, TypedArray};

/// A packaged app's storage, installed into each request's context.
pub(crate) struct StorageHandle(pub(crate) Arc<Storage>);

// This userdata contains no JavaScript values, so changing the marker lifetime
// cannot change its representation or validity.
#[allow(clippy::elidable_lifetime_names)]
unsafe impl<'js> rquickjs::JsLifetime<'js> for StorageHandle {
    type Changed<'to> = StorageHandle;
}

/// The `tokamak:storage` module.
pub(super) struct HostModule;

impl ModuleDef for HostModule {
    fn declare(declarations: &Declarations) -> rquickjs::Result<()> {
        for name in HOST_EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        export_host_functions(ctx, exports)
    }
}

crate::globals::host_functions! {
    pub(super),
    "d1Query" => Async(d1_query),
    "kvGet" => Async(kv_get),
    "kvGetMany" => Async(kv_get_many),
    "kvPut" => Async(kv_put),
    "kvDelete" => Async(kv_delete),
    "kvList" => Async(kv_list),
    "r2Head" => Async(r2_head),
    "r2Get" => Async(r2_get),
    "r2Read" => Async(r2_read),
    "r2CloseBody" => r2_close_body,
    "r2ObjectWriter" => Async(r2_object_writer),
    "r2PartWriter" => Async(r2_part_writer),
    "r2Write" => Async(r2_write),
    "r2Put" => Async(r2_put),
    "r2Delete" => Async(r2_delete),
    "r2List" => Async(r2_list),
    "r2CreateUpload" => Async(r2_create_upload),
    "r2UploadPart" => Async(r2_upload_part),
    "r2CompleteUpload" => Async(r2_complete_upload),
    "r2AbortUpload" => Async(r2_abort_upload),
}

/// A body the R2 binding reads, chunk by chunk.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "R2BodyReader")]
pub(crate) struct Reader {
    #[qjs(skip_trace)]
    body: Arc<Mutex<Option<BodyReader>>>,
}

/// A body the R2 binding writes, chunk by chunk, before recording it.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "R2BodyWriter")]
pub(crate) struct Writer {
    #[qjs(skip_trace)]
    body: Arc<Mutex<Option<BodyWriter>>>,
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
    cache_ttl: Option<i64>,
) -> rquickjs::Result<Option<Object<'_>>> {
    let storage = storage(&ctx)?;
    let entry = blocking(&ctx, move || {
        storage.kv(&binding)?.get(&key, cache_ttl, now())
    })
    .await?;
    entry.map(|entry| entry_object(&ctx, entry)).transpose()
}

/// Resolve to the entry, or null, for each key.
async fn kv_get_many(
    ctx: Ctx<'_>,
    binding: String,
    keys: Vec<String>,
    cache_ttl: Option<i64>,
) -> rquickjs::Result<Vec<Option<Object<'_>>>> {
    let storage = storage(&ctx)?;
    let entries = blocking(&ctx, move || {
        storage.kv(&binding)?.get_many(&keys, cache_ttl, now())
    })
    .await?;
    entries
        .into_iter()
        .map(|entry| entry.map(|entry| entry_object(&ctx, entry)).transpose())
        .collect()
}

/// Store `value` under `key` as the JSON `options` ask.
async fn kv_put<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    value: TypedArray<'js, u8>,
    options: String,
) -> rquickjs::Result<()> {
    let storage = storage(&ctx)?;
    let value = owned_bytes(&ctx, &value)?;
    blocking(&ctx, move || {
        let options = serde_json::from_str(&options).map_err(text)?;
        storage.kv(&binding)?.put(&key, &value, &options, now())
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
    limit: i64,
) -> rquickjs::Result<String> {
    let storage = storage(&ctx)?;
    blocking(&ctx, move || {
        let page = storage.kv(&binding)?.list(&prefix, &cursor, limit, now())?;
        serde_json::to_string(&page).map_err(text)
    })
    .await
}

/// Resolve to the metadata of the object `key` as JSON, or null.
async fn r2_head(ctx: Ctx<'_>, binding: String, key: String) -> rquickjs::Result<Option<String>> {
    in_bucket(&ctx, binding, move |bucket| {
        let object = bucket.head(&key)?;
        Ok(object.as_ref().map(serde_json::to_string).transpose()?)
    })
    .await
}

/// Resolve to `{ object, body }` for the object `key`: its metadata as JSON,
/// and a reader of the range `request` selects unless the object fails the
/// request's conditions. Resolve to null when there is no object.
async fn r2_get(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    request: String,
) -> rquickjs::Result<Option<Object<'_>>> {
    let read = in_bucket(&ctx, binding, move |bucket| {
        bucket.get(&key, &serde_json::from_str(&request)?)
    })
    .await?;
    let (object, body) = match read {
        None => return Ok(None),
        Some(Read::Unmet(object)) => (object, None),
        Some(Read::Body(object, body)) => (object, Some(body)),
    };
    let result = Object::new(ctx.clone())?;
    let object = serde_json::to_string(&object)
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    result.set("object", object)?;
    if let Some(body) = body {
        let body = Arc::new(Mutex::new(Some(body)));
        result.set("body", Class::instance(ctx.clone(), Reader { body })?)?;
    }
    Ok(Some(result))
}

/// Resolve to the next bytes `reader` reads, or null after the last.
async fn r2_read<'js>(
    ctx: Ctx<'js>,
    reader: Class<'js, Reader>,
) -> rquickjs::Result<Option<TypedArray<'js, u8>>> {
    let body = Arc::clone(&reader.borrow().body);
    let chunk = blocking(&ctx, move || {
        let mut body = lock(&body);
        let chunk = match body.as_mut() {
            Some(reader) => reader.read().map_err(text)?,
            None => None,
        };
        if chunk.is_none() {
            body.take();
        }
        Ok(chunk)
    })
    .await?;
    chunk.map(|chunk| TypedArray::new(ctx, chunk)).transpose()
}

/// Close the file `reader` reads.
#[allow(clippy::needless_pass_by_value)]
fn r2_close_body(reader: Class<'_, Reader>) {
    lock(&reader.borrow().body).take();
}

/// Resolve to a writer of a `length`-byte object body, checked against the
/// JSON `checksum` when there is one.
async fn r2_object_writer(
    ctx: Ctx<'_>,
    binding: String,
    length: u64,
    checksum: Option<String>,
) -> rquickjs::Result<Class<'_, Writer>> {
    let body = in_bucket(&ctx, binding, move |bucket| {
        let checksum = checksum.as_deref().map(serde_json::from_str).transpose()?;
        bucket.object_writer(length, checksum)
    })
    .await?;
    writer(ctx, body)
}

/// Resolve to a writer of a `length`-byte part body.
async fn r2_part_writer(
    ctx: Ctx<'_>,
    binding: String,
    length: u64,
) -> rquickjs::Result<Class<'_, Writer>> {
    let body = in_bucket(&ctx, binding, move |bucket| bucket.part_writer(length)).await?;
    writer(ctx, body)
}

/// Append `chunk` to the body `writer` writes.
async fn r2_write<'js>(
    ctx: Ctx<'js>,
    writer: Class<'js, Writer>,
    chunk: TypedArray<'js, u8>,
) -> rquickjs::Result<()> {
    let body = Arc::clone(&writer.borrow().body);
    let bytes = owned_bytes(&ctx, &chunk)?;
    blocking(&ctx, move || {
        lock(&body)
            .as_mut()
            .ok_or_else(|| WRITTEN.to_owned())?
            .write(&bytes)
            .map_err(text)
    })
    .await
}

/// Record the body `writer` wrote as the object `key`, resolving to the
/// object's metadata as JSON, or to null when the object it would replace
/// fails the JSON `request`'s conditions.
async fn r2_put<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    writer: Class<'js, Writer>,
    request: String,
) -> rquickjs::Result<Option<String>> {
    let body = written(&ctx, &writer)?;
    in_bucket(&ctx, binding, move |bucket| {
        let request = serde_json::from_str(&request)?;
        let object = bucket.put(&key, body.finish()?, &request, now_milliseconds())?;
        Ok(object.as_ref().map(serde_json::to_string).transpose()?)
    })
    .await
}

async fn r2_delete(ctx: Ctx<'_>, binding: String, keys: Vec<String>) -> rquickjs::Result<()> {
    in_bucket(&ctx, binding, move |bucket| bucket.delete(&keys)).await
}

/// Resolve to the page of the listing the JSON `request` asks for, as JSON.
async fn r2_list(ctx: Ctx<'_>, binding: String, request: String) -> rquickjs::Result<String> {
    in_bucket(&ctx, binding, move |bucket| {
        let listing = bucket.list(&serde_json::from_str(&request)?)?;
        Ok(serde_json::to_string(&listing)?)
    })
    .await
}

/// Resolve to the ID of a new upload of the object `key`, which gets the
/// JSON `metadata`.
async fn r2_create_upload(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    metadata: String,
) -> rquickjs::Result<String> {
    in_bucket(&ctx, binding, move |bucket| {
        bucket.create_upload(&key, &serde_json::from_str(&metadata)?)
    })
    .await
}

/// Record the body `writer` wrote as part `number` of an upload, resolving to
/// the part's etag.
async fn r2_upload_part<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    upload: String,
    number: i64,
    writer: Class<'js, Writer>,
) -> rquickjs::Result<String> {
    let body = written(&ctx, &writer)?;
    in_bucket(&ctx, binding, move |bucket| {
        bucket.upload_part(&key, &upload, number, body.finish()?)
    })
    .await
}

/// Join the JSON `parts` of an upload into the object `key`, resolving to
/// the object's metadata as JSON.
async fn r2_complete_upload(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    upload: String,
    parts: String,
) -> rquickjs::Result<String> {
    in_bucket(&ctx, binding, move |bucket| {
        let parts: Vec<_> = serde_json::from_str(&parts)?;
        let object = bucket.complete_upload(&key, &upload, &parts, now_milliseconds())?;
        Ok(serde_json::to_string(&object)?)
    })
    .await
}

async fn r2_abort_upload(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    upload: String,
) -> rquickjs::Result<()> {
    in_bucket(&ctx, binding, move |bucket| {
        bucket.abort_upload(&key, &upload)
    })
    .await
}

/// Why a writer has no body.
const WRITTEN: &str = "The body is already recorded";

fn writer(ctx: Ctx<'_>, body: BodyWriter) -> rquickjs::Result<Class<'_, Writer>> {
    let body = Arc::new(Mutex::new(Some(body)));
    Class::instance(ctx, Writer { body })
}

/// The body `writer` wrote, which leaves the writer.
fn written(ctx: &Ctx<'_>, writer: &Class<'_, Writer>) -> rquickjs::Result<BodyWriter> {
    lock(&writer.borrow().body)
        .take()
        .ok_or_else(|| Exception::throw_message(ctx, WRITTEN))
}

/// Run `work` on the bucket the binding `binding` names, on the blocking
/// thread pool, rejecting with its failure.
async fn in_bucket<T: Send + 'static>(
    ctx: &Ctx<'_>,
    binding: String,
    work: impl FnOnce(&R2Bucket) -> Result<T, Failure> + Send + 'static,
) -> rquickjs::Result<T> {
    let storage = storage(ctx)?;
    blocking(ctx, move || work(&*storage.r2(&binding)?).map_err(text)).await
}

/// A copy of `array`'s bytes, which blocking work can own.
fn owned_bytes(ctx: &Ctx<'_>, array: &TypedArray<'_, u8>) -> rquickjs::Result<Vec<u8>> {
    array
        .as_bytes()
        .map(<[u8]>::to_vec)
        .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))
}

fn entry_object<'js>(ctx: &Ctx<'js>, entry: Entry) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("value", TypedArray::new(ctx.clone(), entry.value)?)?;
    object.set("metadata", entry.metadata)?;
    Ok(object)
}

/// Run `work` on the blocking thread pool, rejecting with its error.
///
/// Every operation shares one blocking task type, so `work`'s output is
/// boxed.
async fn blocking<T: Send + 'static>(
    ctx: &Ctx<'_>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> rquickjs::Result<T> {
    let output = run_blocking(ctx, Box::new(move || Ok(Box::new(work()?)))).await?;
    output
        .downcast()
        .map(|output| *output)
        .map_err(|_| Exception::throw_internal(ctx, "blocking work returned another type"))
}

/// Work for the blocking thread pool, with its output boxed.
type Job = Box<dyn FnOnce() -> Result<Box<dyn Any + Send>, String> + Send>;

async fn run_blocking(ctx: &Ctx<'_>, job: Job) -> rquickjs::Result<Box<dyn Any + Send>> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(text)
        .and_then(|result| result)
        .map_err(|error| Exception::throw_message(ctx, &error))
}

/// Seconds since the Unix epoch.
fn now() -> i64 {
    now_milliseconds() / 1000
}

/// Milliseconds since the Unix epoch.
fn now_milliseconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn storage(ctx: &Ctx<'_>) -> rquickjs::Result<Arc<Storage>> {
    ctx.userdata::<StorageHandle>()
        .map(|handle| Arc::clone(&handle.0))
        .ok_or_else(|| Exception::throw_internal(ctx, "storage is not available"))
}
