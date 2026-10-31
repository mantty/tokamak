//! The Worker Cache API's store, which lives as long as its runtime.

#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokamak_macros::{JsType, host_function};

use rquickjs::{Ctx, Exception, FromJs, IntoJs, TypedArray};

use crate::lock;
use crate::typed::StringPairs;

/// A cached response's status line, headers and response properties.
#[derive(Clone, Debug, FromJs, IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct CacheMetadata {
    status: u16,
    status_text: String,
    headers: StringPairs,
    url: String,
    redirected: bool,
    #[qjs(rename = "type")]
    response_type: String,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    metadata: CacheMetadata,
    body: Vec<u8>,
}

/// A cached response: its metadata and body.
#[derive(IntoJs, JsType)]
pub(crate) struct CachedResponse<'js> {
    metadata: CacheMetadata,
    body: TypedArray<'js, u8>,
}

/// The entries of each named cache, by request URL.
#[derive(Debug, Default)]
pub(crate) struct Caches(Mutex<HashMap<String, HashMap<String, CacheEntry>>>);

/// The runtime's caches, installed into each request's context.
struct CachesHandle(Arc<Caches>);

// This userdata contains no JavaScript values, so changing the marker lifetime
// cannot change its representation or validity.
#[allow(clippy::elidable_lifetime_names)]
unsafe impl<'js> rquickjs::JsLifetime<'js> for CachesHandle {
    type Changed<'to> = CachesHandle;
}

/// Gives the request running in `ctx` the runtime's `caches`.
pub(crate) fn attach(ctx: &Ctx<'_>, caches: &Arc<Caches>) -> rquickjs::Result<()> {
    ctx.store_userdata(CachesHandle(Arc::clone(caches)))?;
    Ok(())
}

fn caches(ctx: &Ctx<'_>) -> rquickjs::Result<Arc<Caches>> {
    ctx.userdata::<CachesHandle>()
        .map(|handle| Arc::clone(&handle.0))
        .ok_or_else(|| Exception::throw_internal(ctx, "caches are not available"))
}

#[host_function]
pub(crate) fn cache_match(
    ctx: Ctx<'_>,
    name: String,
    key: String,
) -> rquickjs::Result<Option<CachedResponse<'_>>> {
    let entry = lock(&caches(&ctx)?.0)
        .get(&name)
        .and_then(|entries| entries.get(&key).cloned());
    let Some(CacheEntry { metadata, body }) = entry else {
        return Ok(None);
    };
    Ok(Some(CachedResponse {
        metadata,
        body: TypedArray::new(ctx.clone(), body)?,
    }))
}

#[host_function]
pub(crate) fn cache_put<'js>(
    ctx: Ctx<'js>,
    name: String,
    key: String,
    metadata: CacheMetadata,
    body: TypedArray<'js, u8>,
) -> rquickjs::Result<()> {
    let body = body
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
        .to_vec();
    lock(&caches(&ctx)?.0)
        .entry(name)
        .or_default()
        .insert(key, CacheEntry { metadata, body });
    Ok(())
}

#[host_function]
pub(crate) fn cache_delete(ctx: Ctx<'_>, name: String, key: String) -> rquickjs::Result<bool> {
    let caches = caches(&ctx)?;
    let mut caches = lock(&caches.0);
    let Some(entries) = caches.get_mut(&name) else {
        return Ok(false);
    };
    let deleted = entries.remove(&key).is_some();
    if entries.is_empty() {
        caches.remove(&name);
    }
    Ok(deleted)
}
