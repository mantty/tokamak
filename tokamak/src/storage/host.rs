//! The `tokamak:storage` module: native functions the storage bindings'
//! JavaScript calls. Storage work runs on the blocking thread pool, so it
//! never blocks JavaScript.

use std::any::Any;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokamak_macros::host_function;

use super::kv::{Entry, Page, PutOptions, Value};
use super::r2::{
    BodyReader, BodyWriter, Checksum, Failure, GetRequest, ListRequest, Listing, ObjectMetadata,
    PutRequest, R2Bucket, R2Object, Read, UploadedPart,
};
use super::{Storage, text};
use crate::lock;
use crate::typed::Serde;
use rquickjs::class::Trace;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Class, Ctx, Exception, IntoJs, JsLifetime, TypedArray};
use tokamak_macros::JsType;

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

tokamak_macros::host_functions! {
    pub(super),
    "d1Query" => Async(d1_query),
    "kvGet" => Async(kv_get),
    "kvGetMany" => Async(kv_get_many),
    "kvValue" => kv_value,
    "kvWrite" => kv_write,
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
#[derive(Trace, JsLifetime, tokamak_macros::JsClass)]
#[rquickjs::class(rename = "R2BodyReader")]
pub(crate) struct Reader {
    #[qjs(skip_trace)]
    body: Arc<Mutex<Option<BodyReader>>>,
}

/// A body the R2 binding writes, chunk by chunk, before recording it.
#[derive(Trace, JsLifetime, tokamak_macros::JsClass)]
#[rquickjs::class(rename = "R2BodyWriter")]
pub(crate) struct Writer {
    #[qjs(skip_trace)]
    body: Arc<Mutex<Option<BodyWriter>>>,
}

/// A value the KV binding writes, chunk by chunk, before storing it.
#[derive(Trace, JsLifetime, tokamak_macros::JsClass)]
#[rquickjs::class(rename = "KvValueWriter")]
pub(crate) struct ValueWriter {
    #[qjs(skip_trace)]
    value: Value,
}

/// A D1 service response: its JSON body, and the database's bookmark.
#[derive(IntoJs, JsType)]
pub(crate) struct D1Response {
    body: String,
    bookmark: String,
}

/// Resolve to a D1 service response for `binding`.
#[host_function]
async fn d1_query(
    ctx: Ctx<'_>,
    binding: String,
    format: String,
    body: String,
) -> rquickjs::Result<D1Response> {
    let storage = storage(&ctx)?;
    let (body, bookmark) =
        blocking(&ctx, move || Ok(storage.d1_query(&binding, &body, &format))).await?;
    Ok(D1Response { body, bookmark })
}

/// A KV value and the JSON metadata stored with it.
#[derive(IntoJs, JsType)]
pub(crate) struct KvEntry<'js> {
    value: TypedArray<'js, u8>,
    metadata: Option<String>,
}

impl<'js> KvEntry<'js> {
    fn new(ctx: &Ctx<'js>, entry: Entry) -> rquickjs::Result<Self> {
        Ok(Self {
            value: TypedArray::new(ctx.clone(), entry.value)?,
            metadata: entry.metadata,
        })
    }
}

/// Resolve to the entry of an unexpired key, or undefined.
#[host_function]
async fn kv_get(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    cache_ttl: Option<i64>,
) -> rquickjs::Result<Option<KvEntry<'_>>> {
    let storage = storage(&ctx)?;
    let entry = blocking(&ctx, move || {
        storage.kv(&binding)?.get(&key, cache_ttl, now())
    })
    .await?;
    entry.map(|entry| KvEntry::new(&ctx, entry)).transpose()
}

/// Resolve to the entry, or undefined, for each key.
#[host_function]
async fn kv_get_many(
    ctx: Ctx<'_>,
    binding: String,
    keys: Vec<String>,
    cache_ttl: Option<i64>,
) -> rquickjs::Result<Vec<Option<KvEntry<'_>>>> {
    let storage = storage(&ctx)?;
    let entries = blocking(&ctx, move || {
        storage.kv(&binding)?.get_many(&keys, cache_ttl, now())
    })
    .await?;
    entries
        .into_iter()
        .map(|entry| entry.map(|entry| KvEntry::new(&ctx, entry)).transpose())
        .collect()
}

/// A new value for the KV binding to write.
#[host_function]
fn kv_value(ctx: Ctx<'_>) -> rquickjs::Result<Class<'_, ValueWriter>> {
    let value = Value::default();
    Class::instance(ctx, ValueWriter { value })
}

/// Append `chunk` to the value `writer` writes.
#[allow(clippy::needless_pass_by_value)]
#[host_function]
fn kv_write<'js>(writer: Class<'js, ValueWriter>, chunk: TypedArray<'js, u8>) {
    let bytes = chunk.as_bytes().unwrap_or_default();
    writer.borrow_mut().value.write(bytes);
}

/// Store the value `writer` wrote under `key`, as `options` ask.
#[host_function]
async fn kv_put<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    writer: Class<'js, ValueWriter>,
    options: Serde<PutOptions>,
) -> rquickjs::Result<()> {
    let storage = storage(&ctx)?;
    let value = std::mem::take(&mut writer.borrow_mut().value);
    blocking(&ctx, move || {
        storage.kv(&binding)?.put(&key, &value, &options.0, now())
    })
    .await
}

#[host_function]
async fn kv_delete(ctx: Ctx<'_>, binding: String, key: String) -> rquickjs::Result<()> {
    let storage = storage(&ctx)?;
    blocking(&ctx, move || storage.kv(&binding)?.delete(&key, now())).await
}

/// Resolve to a page of keys.
#[host_function]
async fn kv_list(
    ctx: Ctx<'_>,
    binding: String,
    prefix: String,
    cursor: String,
    limit: i64,
) -> rquickjs::Result<Serde<Page>> {
    let storage = storage(&ctx)?;
    blocking(&ctx, move || {
        storage
            .kv(&binding)?
            .list(&prefix, &cursor, limit, now())
            .map(Serde)
    })
    .await
}

/// Resolve to the metadata of the object `key`, or undefined.
#[host_function]
async fn r2_head(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
) -> rquickjs::Result<Option<Serde<R2Object>>> {
    in_bucket(&ctx, binding, move |bucket| {
        Ok(bucket.head(&key)?.map(Serde))
    })
    .await
}

/// An object a read finds: its metadata, and its body unless the object
/// fails the read's conditions.
#[derive(IntoJs, JsType)]
pub(crate) struct ObjectRead<'js> {
    object: Serde<R2Object>,
    body: Option<ObjectBody<'js>>,
}

/// A reader of the range of an object's body that a read returns, and the
/// range's length.
#[derive(IntoJs, JsType)]
pub(crate) struct ObjectBody<'js> {
    reader: Class<'js, Reader>,
    length: u64,
}

impl<'js> ObjectBody<'js> {
    fn new(ctx: &Ctx<'js>, body: BodyReader) -> rquickjs::Result<Self> {
        let length = body.remaining();
        let body = Arc::new(Mutex::new(Some(body)));
        Ok(Self {
            reader: Class::instance(ctx.clone(), Reader { body })?,
            length,
        })
    }
}

/// Resolve to the object `key` and the range of its body `request` selects,
/// or to undefined when there is no object.
#[host_function]
async fn r2_get(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    request: Serde<GetRequest>,
) -> rquickjs::Result<Option<ObjectRead<'_>>> {
    let read = in_bucket(&ctx, binding, move |bucket| bucket.get(&key, &request.0)).await?;
    let (object, body) = match read {
        None => return Ok(None),
        Some(Read::Unmet(object)) => (object, None),
        Some(Read::Body(object, body)) => (object, Some(ObjectBody::new(&ctx, body)?)),
    };
    Ok(Some(ObjectRead {
        object: Serde(object),
        body,
    }))
}

/// Resolve to the next bytes `reader` reads, or undefined after the last.
#[host_function]
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
#[host_function]
fn r2_close_body(reader: Class<'_, Reader>) {
    lock(&reader.borrow().body).take();
}

/// Resolve to a writer of a `length`-byte object body, checked against
/// `checksum` when there is one.
#[host_function]
async fn r2_object_writer(
    ctx: Ctx<'_>,
    binding: String,
    length: u64,
    checksum: Option<Serde<Checksum>>,
) -> rquickjs::Result<Class<'_, Writer>> {
    let checksum = checksum.map(|checksum| checksum.0);
    let body = in_bucket(&ctx, binding, move |bucket| {
        bucket.object_writer(length, checksum)
    })
    .await?;
    writer(ctx, body)
}

/// Resolve to a writer of a `length`-byte part body.
#[host_function]
async fn r2_part_writer(
    ctx: Ctx<'_>,
    binding: String,
    length: u64,
) -> rquickjs::Result<Class<'_, Writer>> {
    let body = in_bucket(&ctx, binding, move |bucket| bucket.part_writer(length)).await?;
    writer(ctx, body)
}

/// Append `chunk` to the body `writer` writes.
#[host_function]
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
/// object's metadata, or to undefined when the object it would replace fails
/// `request`'s conditions.
#[host_function]
async fn r2_put<'js>(
    ctx: Ctx<'js>,
    binding: String,
    key: String,
    writer: Class<'js, Writer>,
    request: Serde<PutRequest>,
) -> rquickjs::Result<Option<Serde<R2Object>>> {
    let body = written(&ctx, &writer)?;
    in_bucket(&ctx, binding, move |bucket| {
        let object = bucket.put(&key, body.finish()?, &request.0, now_milliseconds())?;
        Ok(object.map(Serde))
    })
    .await
}

#[host_function]
async fn r2_delete(ctx: Ctx<'_>, binding: String, keys: Vec<String>) -> rquickjs::Result<()> {
    in_bucket(&ctx, binding, move |bucket| bucket.delete(&keys)).await
}

/// Resolve to the page of the listing `request` asks for.
#[host_function]
async fn r2_list(
    ctx: Ctx<'_>,
    binding: String,
    request: Serde<ListRequest>,
) -> rquickjs::Result<Serde<Listing>> {
    in_bucket(&ctx, binding, move |bucket| {
        bucket.list(&request.0).map(Serde)
    })
    .await
}

/// Resolve to the ID of a new upload of the object `key`, which gets
/// `metadata`.
#[host_function]
async fn r2_create_upload(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    metadata: Serde<ObjectMetadata>,
) -> rquickjs::Result<String> {
    in_bucket(&ctx, binding, move |bucket| {
        bucket.create_upload(&key, &metadata.0)
    })
    .await
}

/// Record the body `writer` wrote as part `number` of an upload, resolving to
/// the part's etag.
#[host_function]
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

/// Join the `parts` of an upload into the object `key`, resolving to the
/// object's metadata.
#[host_function]
async fn r2_complete_upload(
    ctx: Ctx<'_>,
    binding: String,
    key: String,
    upload: String,
    parts: Serde<Vec<UploadedPart>>,
) -> rquickjs::Result<Serde<R2Object>> {
    in_bucket(&ctx, binding, move |bucket| {
        bucket
            .complete_upload(&key, &upload, &parts.0, now_milliseconds())
            .map(Serde)
    })
    .await
}

#[host_function]
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
        .ok_or_else(|| crate::errors::throw_message(ctx, WRITTEN))
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
        .map_err(|error| crate::errors::throw_message(ctx, &error))
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
