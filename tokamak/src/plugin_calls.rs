//! Plugin calls from the Worker. The shell's plugins answer them, apart from
//! `tokamak.lifecycleStage`, which the runtime answers itself, and methods the
//! shell does not implement, which it rejects.
//!
//! Payloads are opaque bytes; reply metadata controls completion.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::json;
use tokio::sync::{Notify, mpsc, oneshot};

use crate::lock;

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
    /// Whether the shell implements `method` of `plugin`. The runtime rejects
    /// calls and subscriptions to the methods it does not implement with
    /// `NotSupportedError`, without passing them to the shell.
    fn implements(&self, _plugin: &str, _method: &str) -> bool {
        true
    }

    /// Calls `method` of `plugin` with its encoded arguments.
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]);

    /// Calls `method` of `plugin` with its encoded arguments and a body of
    /// bytes, whose chunks the shell pulls one at a time with
    /// [`crate::Runtime::pull`].
    fn call_with_body(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]);

    /// Passes the chunk of the call `id`'s body that the shell pulled.
    fn chunk(&self, id: u64, chunk: &PluginReply);

    /// Subscribes to `method` of `plugin` with its encoded arguments.
    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]);

    /// Cancels the pending call or subscription `id`, releasing its resources.
    fn cancel(&self, id: u64);
}

/// Encoded bytes or an encoded error, and whether they are the last.
///
/// A native endpoint's results, and the chunks of a call's body, travel as
/// these.
#[derive(Clone, Debug)]
pub struct PluginReply {
    /// Endpoint-owned encoding, passed through the host without interpretation.
    pub payload: Vec<u8>,
    /// Whether the payload encodes an error.
    pub failed: bool,
    /// Whether the call, subscription or body has finished.
    pub done: bool,
}

/// How a subscription treats a result that arrives while its queue is full.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Overflow {
    /// Ends the subscription with `QuotaExceededError` and cancels its work.
    #[default]
    Error,
    /// Discards the oldest queued result.
    DropOldest,
}

impl Overflow {
    /// The policy named `name` in the native protocol.
    pub(crate) fn named(name: &str) -> Option<Self> {
        match name {
            "error" => Some(Self::Error),
            "dropOldest" => Some(Self::DropOldest),
            _ => None,
        }
    }
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
    Call {
        result: oneshot::Sender<PluginReply>,
        /// Requests for the next chunk of the call's body.
        pulls: Option<mpsc::UnboundedSender<()>>,
    },
    Subscription {
        results: Arc<Results>,
        overflow: Overflow,
    },
}

/// A subscription's results, which it ends when dropped.
pub(crate) struct Subscription {
    // Drops first, so the shell is cancelled while it still has a reader.
    _shell: Option<Registration>,
    results: Arc<Results>,
}

/// The body of a call, whose chunks the shell pulls one at a time.
pub(crate) struct Body {
    calls: Arc<PluginCalls>,
    id: u64,
    pulls: mpsc::UnboundedReceiver<()>,
}

/// A call or subscription the shell runs, which stops waiting for results
/// when dropped.
struct Registration {
    calls: Arc<PluginCalls>,
    id: u64,
}

/// Results a subscription's reader has not taken yet.
#[derive(Default)]
struct Results {
    queue: Mutex<(VecDeque<PluginReply>, bool)>,
    ready: Notify,
}

/// What queueing a subscription's result did to the subscription.
#[derive(PartialEq, Eq)]
enum Queued {
    Continues,
    Finished,
    Overflowed,
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
        let waiting = Waiting::Call {
            result: sender,
            pulls: None,
        };
        let registration = self.start(plugin, method, waiting, |handler, id| {
            handler.call(id, plugin, method, arguments);
        });
        settle(registration, receiver)
    }

    /// Calls `method` of `plugin` with a body, resolving to its encoded
    /// result. The body is `None` when the runtime answers the call itself.
    pub(crate) fn call_with_body(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &[u8],
    ) -> (
        impl Future<Output = PluginReply> + Send + 'static,
        Option<Body>,
    ) {
        let (sender, receiver) = oneshot::channel();
        let (pulls, pulled) = mpsc::unbounded_channel();
        let waiting = Waiting::Call {
            result: sender,
            pulls: Some(pulls),
        };
        let registration = self.start(plugin, method, waiting, |handler, id| {
            handler.call_with_body(id, plugin, method, arguments);
        });
        let body = registration.as_ref().map(|registration| Body {
            calls: Arc::clone(self),
            id: registration.id,
            pulls: pulled,
        });
        (settle(registration, receiver), body)
    }

    /// Subscribes to `method` of `plugin`, treating a full queue as
    /// `overflow` says.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        plugin: &str,
        method: &str,
        arguments: &[u8],
        overflow: Overflow,
    ) -> Subscription {
        let results = Arc::new(Results::default());
        let waiting = Waiting::Subscription {
            results: Arc::clone(&results),
            overflow,
        };
        let shell = self.start(plugin, method, waiting, |handler, id| {
            handler.subscribe(id, plugin, method, arguments);
        });
        Subscription {
            _shell: shell,
            results,
        }
    }

    /// Passes the shell's encoded result to the call or subscription.
    pub(crate) fn reply(&self, id: u64, result: PluginReply) {
        let mut waiting = self.waiting();
        let queued = match waiting.get(&id) {
            Some(Waiting::Subscription { results, overflow }) => results.push(result, *overflow),
            Some(Waiting::Call { .. }) => {
                let Some(call) = waiting.remove(&id) else {
                    return;
                };
                call.send(result);
                return;
            }
            None => return,
        };
        if queued == Queued::Continues {
            return;
        }
        waiting.remove(&id);
        drop(waiting);
        if queued == Queued::Overflowed
            && let Some(handler) = &self.handler
        {
            handler.cancel(id);
        }
    }

    /// Asks the Worker for the next chunk of the call `id`'s body, which
    /// arrives through [`PluginHandler::chunk`].
    pub(crate) fn pull(&self, id: u64) {
        if let Some(Waiting::Call {
            pulls: Some(pulls), ..
        }) = self.waiting().get(&id)
        {
            let _ = pulls.send(());
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

    /// The shell's handler, unless the runtime answers calls to `method` of
    /// `plugin`.
    fn shell(&self, plugin: &str, method: &str) -> Option<&Arc<dyn PluginHandler>> {
        if plugin == RUNTIME_PLUGIN && method == "lifecycleStage" {
            None
        } else {
            self.handler
                .as_ref()
                .filter(|handler| handler.implements(plugin, method))
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
        lock(&self.waiting)
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

/// The result of the call `registration` runs, which stops waiting when
/// dropped.
async fn settle(
    registration: Option<Registration>,
    result: oneshot::Receiver<PluginReply>,
) -> PluginReply {
    let _registration = registration;
    result
        .await
        .unwrap_or_else(|_| error("AbortError", "the runtime stopped"))
}

impl Waiting {
    fn send(self, result: PluginReply) {
        match self {
            Self::Call { result: sender, .. } => drop(sender.send(result)),
            Self::Subscription { results, overflow } => drop(results.push(result, overflow)),
        }
    }
}

impl Results {
    /// Queues `result`, or applies `overflow` when the queue is full.
    fn push(&self, result: PluginReply, overflow: Overflow) -> Queued {
        let mut queue = lock(&self.queue);
        let (results, finished) = &mut *queue;
        let queued = if result.done {
            results.push_back(result);
            Queued::Finished
        } else if results.len() < SUBSCRIPTION_CAPACITY {
            results.push_back(result);
            Queued::Continues
        } else if overflow == Overflow::DropOldest {
            results.pop_front();
            results.push_back(result);
            Queued::Continues
        } else {
            results.push_back(error(
                "QuotaExceededError",
                "Native subscription consumer is too slow",
            ));
            Queued::Overflowed
        };
        *finished = queued != Queued::Continues;
        drop(queue);
        self.ready.notify_one();
        queued
    }

    /// The next result, or `None` once the last has been taken.
    async fn next(&self) -> Option<PluginReply> {
        loop {
            let ready = self.ready.notified();
            let (next, finished) = {
                let mut queue = lock(&self.queue);
                (queue.0.pop_front(), queue.1)
            };
            if next.is_some() || finished {
                return next;
            }
            ready.await;
        }
    }
}

impl Subscription {
    /// The next result, or `None` once the last was `done`.
    pub(crate) async fn next(&mut self) -> Option<PluginReply> {
        self.results.next().await
    }
}

impl Body {
    /// Waits until the shell pulls the next chunk, returning false once the
    /// call has ended.
    pub(crate) async fn pulled(&mut self) -> bool {
        self.pulls.recv().await.is_some()
    }

    /// Passes the chunk the shell pulled: bytes, or an encoded error when
    /// `failed`, and `done` with the last.
    pub(crate) fn send(&self, chunk: &PluginReply) {
        if let Some(handler) = &self.calls.handler {
            handler.chunk(self.id, chunk);
        }
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
