//! Plugin calls from the Worker. The shell's plugins answer them, apart from
//! calls to the reserved plugin `tokamak`, which the runtime answers itself.
//!
//! Payloads are opaque bytes; reply metadata controls completion.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde_json::json;
use tokio::sync::{mpsc, oneshot};

/// The plugin ID the runtime answers calls to itself.
const RUNTIME_PLUGIN: &str = "tokamak";
const SUBSCRIPTION_CAPACITY: usize = 64;

/// The shell's plugins, which run the Worker's calls and subscriptions.
///
/// The shell answers each with [`crate::Runtime::reply`]: once for a call,
/// and once per value for a subscription until a result is `done` or the
/// runtime cancels. The runtime calls these methods on its own threads,
/// and they must return promptly.
pub trait PluginHandler: Send + Sync {
    /// Calls `method` of `plugin` with its encoded arguments.
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]);

    /// Subscribes to `method` of `plugin` with its encoded arguments.
    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]);

    /// Cancels the pending call or subscription `id`, releasing its resources.
    fn cancel(&self, id: u64);
}

/// A native endpoint's encoded value or error and completion status.
#[derive(Clone, Debug)]
pub struct PluginReply {
    /// Endpoint-owned encoding, passed through the host without interpretation.
    pub payload: Vec<u8>,
    /// Whether the payload encodes an error.
    pub failed: bool,
    /// Whether the call or subscription has finished.
    pub done: bool,
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
    Call(oneshot::Sender<PluginReply>),
    Subscription {
        sender: mpsc::Sender<PluginReply>,
        overflow: oneshot::Sender<PluginReply>,
    },
}

/// A subscription's results, which it ends when dropped.
pub(crate) struct Subscription {
    // Drops before `results`, so the shell is cancelled while it still has
    // a reader.
    _shell: Option<Registration>,
    results: mpsc::Receiver<PluginReply>,
    overflow: Option<oneshot::Receiver<PluginReply>>,
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

    /// Calls `method` of `plugin`, resolving to its encoded result.
    pub(crate) fn call(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &[u8],
    ) -> impl Future<Output = PluginReply> + Send + 'static {
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

    /// Subscribes to `method` of `plugin`.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &[u8],
    ) -> Subscription {
        let (sender, results) = mpsc::channel(SUBSCRIPTION_CAPACITY);
        let (overflow, failure) = oneshot::channel();
        let shell = self.start(
            plugin,
            method,
            Waiting::Subscription { sender, overflow },
            |handler, id| {
                handler.subscribe(id, plugin, method, arguments);
            },
        );
        Subscription {
            _shell: shell,
            results,
            overflow: Some(failure),
        }
    }

    /// Passes the shell's encoded result to the call or subscription.
    pub(crate) fn reply(&self, id: u64, result: PluginReply) {
        let done = result.done;
        let mut waiting = self.waiting();
        let Some(destination) = waiting.remove(&id) else {
            return;
        };
        match destination {
            Waiting::Call(sender) => {
                let _ = sender.send(result);
            }
            Waiting::Subscription { sender, overflow } => match sender.try_send(result) {
                Ok(()) if !done => {
                    waiting.insert(id, Waiting::Subscription { sender, overflow });
                }
                Ok(()) => {}
                Err(failure) => {
                    if matches!(failure, mpsc::error::TrySendError::Full(_)) {
                        let _ = overflow.send(error(
                            "QuotaExceededError",
                            "Native subscription consumer is too slow",
                        ));
                    }
                    drop(waiting);
                    if let Some(handler) = &self.handler {
                        handler.cancel(id);
                    }
                }
            },
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
    fn shell(&self, plugin: &str, method: &str) -> Option<&Arc<dyn PluginHandler>> {
        if plugin == RUNTIME_PLUGIN && method == "lifecycleStage" {
            None
        } else {
            self.handler.as_ref()
        }
    }

    /// The runtime's own result for `method` of `plugin`.
    fn answer(&self, plugin: &str, method: &str) -> PluginReply {
        match (plugin, method) {
            (RUNTIME_PLUGIN, "lifecycleStage") => {
                let stage = if self.is_foreground() {
                    "foreground"
                } else {
                    "background"
                };
                PluginReply {
                    payload: json!(stage).to_string().into_bytes(),
                    failed: false,
                    done: true,
                }
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
        let Some(handler) = self.shell(plugin, method) else {
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
    fn send(self, result: PluginReply) {
        match self {
            Self::Call(sender) => drop(sender.send(result)),
            Self::Subscription { sender, .. } => drop(sender.try_send(result)),
        }
    }
}

impl Subscription {
    /// The next result, or `None` once the last was `done`.
    pub(crate) async fn next(&mut self) -> Option<PluginReply> {
        if let Some(result) = self.results.recv().await {
            return Some(result);
        }
        self.overflow.take()?.await.ok()
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let removed = self.calls.waiting().remove(&self.id);
        if let (Some(_), Some(handler)) = (removed, &self.calls.handler) {
            handler.cancel(self.id);
        }
    }
}

/// A failed result, which ends a subscription.
pub(crate) fn error(name: &str, message: &str) -> PluginReply {
    PluginReply {
        payload: json!({ "name": name, "message": message })
            .to_string()
            .into_bytes(),
        failed: true,
        done: true,
    }
}

#[cfg(test)]
mod tests;
