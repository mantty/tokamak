//! The Worker Cache API's store, which lives as long as its runtime.

#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rquickjs::{Ctx, Exception, Object, TypedArray};

/// Ordered header pairs; duplicate names (for example `set-cookie`) are preserved.
type HeaderList = Vec<(String, String)>;

#[derive(Clone, Debug)]
struct CacheEntry {
    status: u16,
    status_text: String,
    headers: HeaderList,
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

impl Caches {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, HashMap<String, CacheEntry>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
    let entry = caches(&ctx)?
        .lock()
        .get(&name)
        .and_then(|entries| entries.get(&key).cloned());
    let Some(entry) = entry else {
        return Ok(None);
    };
    let result = Object::new(ctx.clone())?;
    result.set("status", entry.status)?;
    result.set("statusText", entry.status_text)?;
    result.set(
        "headers",
        serde_json::to_string(&entry.headers)
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?,
    )?;
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
    let headers: String = metadata.get("headers")?;
    let entry = CacheEntry {
        status: metadata.get("status")?,
        status_text: metadata.get("statusText")?,
        headers: serde_json::from_str(&headers).map_err(|error| {
            Exception::throw_type(&ctx, &format!("Invalid cache headers: {error}"))
        })?,
        body: body
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
            .to_vec(),
        url: metadata.get("url")?,
        redirected: metadata.get("redirected")?,
        response_type: metadata.get("type")?,
    };
    caches(&ctx)?
        .lock()
        .entry(name)
        .or_default()
        .insert(key, entry);
    Ok(())
}

pub(crate) fn cache_delete(ctx: Ctx<'_>, name: String, key: String) -> rquickjs::Result<bool> {
    let caches = caches(&ctx)?;
    let mut caches = caches.lock();
    let Some(entries) = caches.get_mut(&name) else {
        return Ok(false);
    };
    let deleted = entries.remove(&key).is_some();
    if entries.is_empty() {
        caches.remove(&name);
    }
    Ok(deleted)
}
