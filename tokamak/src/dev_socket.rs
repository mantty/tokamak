//! The dev WebSocket between the development app and the tokamak Vite plugin,
//! through `tok dev`'s relay. It carries the app's events to the development
//! Worker, and the development Worker's plugin calls to the app.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use crate::gateway::HandlerError;
use crate::plugin_calls::PluginCalls;
use crate::transport::{parse_websocket_frame, write_frame};

/// The development app's end of the dev WebSocket, while it is open.
pub(crate) struct DevSocket {
    connection: watch::Sender<Option<Arc<Connection>>>,
    next_id: AtomicU64,
    plugins: Arc<PluginCalls>,
}

/// One open dev WebSocket.
struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    /// Where each delivery waiting for its answer receives it, by message ID.
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>,
}

/// An event the app sends.
#[derive(Serialize)]
struct EventMessage<'a> {
    r#type: &'static str,
    id: u64,
    name: &'a str,
    event: &'a RawValue,
}

/// A message the plugin sends: the reply to the event `id` or why it failed,
/// or a call, subscription or unsubscription of the development Worker.
#[derive(Deserialize)]
struct Received<'a> {
    r#type: &'a str,
    id: u64,
    #[serde(borrow)]
    reply: Option<&'a RawValue>,
    message: Option<String>,
    plugin: Option<String>,
    method: Option<String>,
    #[serde(borrow)]
    arguments: Option<&'a RawValue>,
}

impl DevSocket {
    pub(crate) fn new(plugins: Arc<PluginCalls>) -> Self {
        Self {
            connection: watch::Sender::new(None),
            next_id: AtomicU64::new(0),
            plugins,
        }
    }

    /// Sends the event `name`, whose JSON is `event`, once the dev WebSocket
    /// is open, and returns the JSON reply the plugin answers with by
    /// `deadline`.
    pub(crate) async fn deliver(
        &self,
        name: &str,
        event: &str,
        deadline: Instant,
    ) -> Result<String, HandlerError> {
        let deadline = tokio::time::Instant::from_std(deadline);
        let late = || format!("{name} did not complete before its deadline");
        let closed = || format!("the dev socket closed before {name} completed");
        let mut connections = self.connection.subscribe();
        let connection = tokio::time::timeout_at(deadline, connections.wait_for(Option::is_some))
            .await
            .map_err(|_| late())??
            .clone()
            .ok_or_else(late)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message = serde_json::to_string(&EventMessage {
            r#type: "event",
            id,
            name,
            event: serde_json::from_str(event)?,
        })?;
        let (sender, answer) = oneshot::channel();
        connection.pending().insert(id, sender);
        if connection.outgoing.send(message).is_err() {
            connection.pending().remove(&id);
            return Err(closed().into());
        }
        let answer = tokio::time::timeout_at(deadline, answer).await;
        connection.pending().remove(&id);
        match answer {
            Err(_) => Err(late().into()),
            Ok(Err(_)) => Err(closed().into()),
            Ok(Ok(answer)) => answer.map_err(Into::into),
        }
    }

    /// Relays `upgraded`'s frames until it closes or `stopped` is cancelled,
    /// delivering events and running the development Worker's plugin calls
    /// through it meanwhile. Its subscriptions end with it.
    pub(crate) async fn serve(&self, upgraded: Upgraded, stopped: &CancellationToken) {
        let (outgoing, messages) = mpsc::unbounded_channel();
        let connection = Arc::new(Connection {
            outgoing,
            pending: Mutex::default(),
        });
        self.connection.send_replace(Some(Arc::clone(&connection)));
        let mut subscriptions = HashMap::new();
        let _ = relay(upgraded, messages, stopped, |message| {
            self.receive(&connection, &mut subscriptions, message);
        })
        .await;
        self.connection.send_replace(None);
        connection.pending().clear();
        for subscription in subscriptions.values() {
            subscription.abort();
        }
    }

    /// Passes the plugin's `message` to the delivery it answers, or runs the
    /// call it carries, sending its results on `connection`.
    fn receive(
        &self,
        connection: &Connection,
        subscriptions: &mut HashMap<u64, AbortHandle>,
        message: &[u8],
    ) {
        let Ok(message) = serde_json::from_slice::<Received>(message) else {
            return;
        };
        let id = message.id;
        let plugin = message.plugin.as_deref().unwrap_or_default();
        let method = message.method.as_deref().unwrap_or_default();
        let arguments = message.arguments.map_or("null", RawValue::get);
        match message.r#type {
            "reply" => connection.answer(id, Ok(message.reply.map_or("null", RawValue::get))),
            "error" => connection.answer(id, Err(message.message.as_deref().unwrap_or_default())),
            "call" => {
                let result = self.plugins.call(plugin, method, arguments);
                let outgoing = connection.outgoing.clone();
                tokio::spawn(async move { send_result(&outgoing, id, &result.await) });
            }
            "subscribe" => {
                let mut subscription = self.plugins.subscribe(plugin, method, arguments);
                let outgoing = connection.outgoing.clone();
                let task = tokio::spawn(async move {
                    while let Some(result) = subscription.next().await {
                        if !send_result(&outgoing, id, &result) {
                            return;
                        }
                    }
                });
                subscriptions.retain(|_, subscription| !subscription.is_finished());
                subscriptions.insert(id, task.abort_handle());
            }
            "unsubscribe" => {
                if let Some(subscription) = subscriptions.remove(&id) {
                    subscription.abort();
                }
            }
            _ => {}
        }
    }
}

/// Sends the plugin's JSON `result` for the call or subscription `id`,
/// returning whether the dev WebSocket is still open.
fn send_result(outgoing: &mpsc::UnboundedSender<String>, id: u64, result: &str) -> bool {
    let message = format!(r#"{{"type":"result","id":{id},"result":{result}}}"#);
    outgoing.send(message).is_ok()
}

impl Connection {
    fn pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<String, String>>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Passes the plugin's `answer` to the delivery of the event `id`.
    fn answer(&self, id: u64, answer: Result<&str, &str>) {
        if let Some(sender) = self.pending().remove(&id) {
            let _ = sender.send(answer.map(str::to_owned).map_err(str::to_owned));
        }
    }
}

/// Relays frames between `upgraded` and the app, passing each message the
/// plugin sends to `receive` and sending each of `messages`, until either
/// side closes or `stopped` is cancelled.
async fn relay(
    upgraded: Upgraded,
    mut messages: mpsc::UnboundedReceiver<String>,
    stopped: &CancellationToken,
    mut receive: impl FnMut(&[u8]),
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(TokioIo::new(upgraded));
    let mut buffer = Vec::new();
    let mut bytes = [0; 8192];
    // The text of a message whose frames have not all arrived.
    let mut message = Vec::new();
    loop {
        while let Some(frame) = parse_websocket_frame(&mut buffer, false)? {
            match frame.opcode {
                0x0 | 0x1 => message.extend_from_slice(&frame.payload),
                0x8 => return Ok(()),
                0x9 => write_frame(&mut writer, 0xA, &frame.payload).await?,
                _ => continue,
            }
            if frame.final_frame && matches!(frame.opcode, 0x0 | 0x1) {
                receive(&std::mem::take(&mut message));
            }
        }
        tokio::select! {
            () = stopped.cancelled() => return Ok(()),
            message = messages.recv() => match message {
                Some(message) => write_frame(&mut writer, 0x1, message.as_bytes()).await?,
                None => return Ok(()),
            },
            count = reader.read(&mut bytes) => match count? {
                0 => return Ok(()),
                count => buffer.extend_from_slice(&bytes[..count]),
            },
        }
    }
}
