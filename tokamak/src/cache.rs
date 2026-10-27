//! The Worker Cache API's store, which lives as long as its runtime.

#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rquickjs::{Ctx, Exception, Object, TypedArray};

use crate::lock;

#[derive(Clone, Debug)]
struct CacheEntry {
    status: u16,
    status_text: String,
    /// The JSON array of header pairs the Worker's `Cache.put` sent.
    headers: String,
    body: Vec<u8>,
    url: String,
    redirected: bool,
    response_type: String,
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

pub(crate) fn cache_match(
    ctx: Ctx<'_>,
    name: String,
    key: String,
) -> rquickjs::Result<Option<Object<'_>>> {
    let entry = lock(&caches(&ctx)?.0)
        .get(&name)
        .and_then(|entries| entries.get(&key).cloned());
    let Some(entry) = entry else {
        return Ok(None);
    };
    let result = Object::new(ctx.clone())?;
    result.set("status", entry.status)?;
    result.set("statusText", entry.status_text)?;
    result.set("headers", entry.headers)?;
    result.set("body", TypedArray::new(ctx.clone(), entry.body)?)?;
    result.set("url", entry.url)?;
    result.set("redirected", entry.redirected)?;
    result.set("type", entry.response_type)?;
    Ok(Some(result))
}

pub(crate) fn cache_put<'js>(
    ctx: Ctx<'js>,
    name: String,
    key: String,
    metadata: Object<'js>,
    body: TypedArray<'js, u8>,
) -> rquickjs::Result<()> {
    let entry = CacheEntry {
        status: metadata.get("status")?,
        status_text: metadata.get("statusText")?,
        headers: metadata.get("headers")?,
        body: body
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
            .to_vec(),
        url: metadata.get("url")?,
        redirected: metadata.get("redirected")?,
        response_type: metadata.get("type")?,
    };
    lock(&caches(&ctx)?.0)
        .entry(name)
        .or_default()
        .insert(key, entry);
    Ok(())
}

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
