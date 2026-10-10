//! Plugin calls from the Worker. The shell's plugins answer them, apart from
//! calls to the reserved plugin `tokamak`, which the runtime answers itself.
//!
//! A result is the JSON object the page's native transport receives, without
//! its session and ID: `{"value": …, "done": …}` or
//! `{"error": {"name": …, "message": …}, "done": …}`. `done` ends a
//! subscription.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

/// The plugin ID the runtime answers calls to itself.
const RUNTIME_PLUGIN: &str = "tokamak";

/// The shell's plugins, which run the Worker's calls and subscriptions.
///
/// The shell answers each with [`crate::Runtime::reply`]: once for a call,
/// and once per value for a subscription until a result is `done` or the
/// runtime unsubscribes. The runtime calls these methods on its own threads,
/// and they must return promptly.
pub trait PluginHandler: Send + Sync {
    /// Calls `method` of `plugin` with the JSON `arguments`.
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &str);

    /// Subscribes to `method` of `plugin` with the JSON `arguments`.
    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &str);

    /// Ends the subscription `id`.
    fn unsubscribe(&self, id: u64);
}

impl std::fmt::Debug for dyn PluginHandler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PluginHandler")
    }
}

/// The Worker's calls waiting for their results, and the app's lifecycle
/// stage, which `tokamak.lifecycleStage` answers.
pub(crate) struct PluginCalls {
    handler: Option<Arc<dyn PluginHandler>>,
    foreground: AtomicBool,
    next_id: AtomicU64,
    waiting: Mutex<HashMap<u64, Waiting>>,
}

/// Where the results of a call or subscription go.
enum Waiting {
    Call(oneshot::Sender<String>),
    Subscription(mpsc::UnboundedSender<String>),
}

/// The part of a result that ends a subscription.
#[derive(Deserialize)]
struct Done {
    #[serde(default)]
    done: bool,
}

/// A subscription's results, which it ends when dropped.
pub(crate) struct Subscription {
    // Drops before `results`, so the shell is unsubscribed while it still has
    // a reader.
    _shell: Option<Registration>,
    results: mpsc::UnboundedReceiver<String>,
}

/// A call or subscription the shell runs, which stops waiting for results
/// when dropped.
struct Registration {
    calls: Arc<PluginCalls>,
    id: u64,
}

impl PluginCalls {
    pub(crate) fn new(handler: Option<Arc<dyn PluginHandler>>, foreground: bool) -> Arc<Self> {
        Arc::new(Self {
            handler,
            foreground: AtomicBool::new(foreground),
            next_id: AtomicU64::new(1),
            waiting: Mutex::default(),
        })
    }

    /// Calls `method` of `plugin` with the JSON `arguments`, resolving to its
    /// result.
    pub(crate) fn call(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &str,
    ) -> impl Future<Output = String> + Send + 'static {
        let (sender, receiver) = oneshot::channel();
        let registration = self.start(plugin, method, Waiting::Call(sender), |handler, id| {
            handler.call(id, plugin, method, arguments);
        });
        async move {
            let _registration = registration;
            receiver
                .await
                .unwrap_or_else(|_| error("AbortError", "the runtime stopped"))
        }
    }

    /// Subscribes to `method` of `plugin` with the JSON `arguments`.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &str,
    ) -> Subscription {
        let (sender, results) = mpsc::unbounded_channel();
        let shell = self.start(
            plugin,
            method,
            Waiting::Subscription(sender),
            |handler, id| {
                handler.subscribe(id, plugin, method, arguments);
            },
        );
        Subscription {
            _shell: shell,
            results,
        }
    }

    /// Passes the shell's `result` to the call or subscription `id`. A
    /// result that is not a JSON object is replaced with an error.
    pub(crate) fn reply(&self, id: u64, result: &str) {
        let (result, done) = match serde_json::from_str::<Done>(result) {
            Ok(Done { done }) => (result.to_owned(), done),
            Err(_) => (not_json(), false),
        };
        let mut waiting = self.waiting();
        let Some(destination) = waiting.remove(&id) else {
            return;
        };
        match destination {
            Waiting::Call(sender) => {
                let _ = sender.send(result);
            }
            Waiting::Subscription(sender) => {
                let _ = sender.send(result);
                if !done {
                    waiting.insert(id, Waiting::Subscription(sender));
                }
            }
        }
    }

    /// Records whether the app is in the foreground, returning whether that
    /// changed.
    pub(crate) fn set_foreground(&self, foreground: bool) -> bool {
        self.foreground.swap(foreground, Ordering::AcqRel) != foreground
    }

    pub(crate) fn is_foreground(&self) -> bool {
        self.foreground.load(Ordering::Acquire)
    }

    /// The shell's handler, unless the runtime answers calls to `plugin`.
    fn shell(&self, plugin: &str) -> Option<&Arc<dyn PluginHandler>> {
        if plugin == RUNTIME_PLUGIN {
            None
        } else {
            self.handler.as_ref()
        }
    }

    /// The runtime's own result for `method` of `plugin`.
    fn answer(&self, plugin: &str, method: &str) -> String {
        match (plugin, method) {
            (RUNTIME_PLUGIN, "lifecycleStage") => {
                let stage = if self.is_foreground() {
                    "foreground"
                } else {
                    "background"
                };
                json!({ "value": stage, "done": true }).to_string()
            }
            _ => error(
                "NotSupportedError",
                &format!("{plugin}.{method} is not supported"),
            ),
        }
    }

    /// Passes a call or subscription to the shell with `request`, registered
    /// to receive its results in `waiting`, or answers it at once.
    fn start(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        waiting: Waiting,
        request: impl FnOnce(&dyn PluginHandler, u64),
    ) -> Option<Registration> {
        let Some(handler) = self.shell(plugin) else {
            waiting.send(self.answer(plugin, method));
            return None;
        };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.waiting().insert(id, waiting);
        request(handler.as_ref(), id);
        Some(Registration {
            calls: Arc::clone(self),
            id,
        })
    }

    fn waiting(&self) -> MutexGuard<'_, HashMap<u64, Waiting>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for PluginCalls {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginCalls")
            .field("handler", &self.handler)
            .field("foreground", &self.foreground)
            .finish_non_exhaustive()
    }
}

impl Waiting {
    fn send(self, result: String) {
        match self {
            Self::Call(sender) => drop(sender.send(result)),
            Self::Subscription(sender) => drop(sender.send(result)),
        }
    }
}

impl Subscription {
    /// The next result, or `None` once the last was `done`.
    pub(crate) async fn next(&mut self) -> Option<String> {
        self.results.recv().await
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let removed = self.calls.waiting().remove(&self.id);
        if let (Some(Waiting::Subscription(_)), Some(handler)) = (removed, &self.calls.handler) {
            handler.unsubscribe(self.id);
        }
    }
}

/// A failed result, which ends a subscription.
pub(crate) fn error(name: &str, message: &str) -> String {
    json!({ "error": { "name": name, "message": message }, "done": true }).to_string()
}

/// The error that replaces a result that is not JSON, which leaves a
/// subscription running.
fn not_json() -> String {
    json!({
        "error": { "name": "OperationError", "message": "the plugin's result is not JSON" },
        "done": false,
    })
    .to_string()
}

#[cfg(test)]
mod tests;
