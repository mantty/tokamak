use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
pub(crate) mod brotli;
pub(crate) mod headers;
pub(crate) mod http;
pub(crate) mod sockets;

/// Ordered header pairs; duplicate names (for example `set-cookie`) are preserved.
pub(crate) type HeaderList = Vec<(String, String)>;

#[derive(Clone)]
pub(crate) struct CacheEntry {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: HeaderList,
    pub(crate) body: Vec<u8>,
    pub(crate) url: String,
    pub(crate) redirected: bool,
    pub(crate) response_type: String,
}

type CacheStore = HashMap<String, HashMap<String, CacheEntry>>;
static CACHES: OnceLock<Mutex<CacheStore>> = OnceLock::new();

fn caches() -> &'static Mutex<CacheStore> {
    CACHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_scope(path: &str, name: &str) -> String {
    format!("{path}\0{name}")
}

pub(crate) fn cache_match(path: &str, name: &str, key: &str) -> Option<CacheEntry> {
    caches()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_scope(path, name))
        .and_then(|entries| entries.get(key).cloned())
}

pub(crate) fn cache_put(path: &str, name: &str, key: String, entry: CacheEntry) {
    caches()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entry(cache_scope(path, name))
        .or_default()
        .insert(key, entry);
}

pub(crate) fn cache_delete(path: &str, name: &str, key: &str) -> bool {
    let mut caches = caches()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scope = cache_scope(path, name);
    let Some(entries) = caches.get_mut(&scope) else {
        return false;
    };
    let deleted = entries.remove(key).is_some();
    if entries.is_empty() {
        caches.remove(&scope);
    }
    deleted
}
