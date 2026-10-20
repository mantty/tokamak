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
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;

use crate::gateway::HandlerError;
use crate::native_protocol::{self, Header};
use crate::plugin_calls::{PluginCalls, PluginReply};
use crate::transport::{parse_websocket_frame, write_frame};

/// The development app's end of the dev WebSocket, while it is open.
pub(crate) struct DevSocket {
    connection: watch::Sender<Option<Arc<Connection>>>,
    next_id: AtomicU64,
    plugins: Arc<PluginCalls>,
}

/// One open dev WebSocket.
struct Connection {
    outgoing: mpsc::Sender<(Header, Vec<u8>)>,
    /// Where each delivery waiting for its answer receives it, by message ID.
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>,
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
        let message = (
            Header {
                r#type: "event".into(),
                id,
                name: Some(name.into()),
                ..Header::default()
            },
            event.as_bytes().to_vec(),
        );
        let (sender, answer) = oneshot::channel();
        connection.pending().insert(id, sender);
        if !matches!(
            tokio::time::timeout_at(deadline, connection.outgoing.send(message)).await,
            Ok(Ok(()))
        ) {
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
    /// through it meanwhile. Its pending calls and subscriptions end with it.
    pub(crate) async fn serve(&self, upgraded: Upgraded, stopped: &CancellationToken) {
        let (outgoing, messages) = mpsc::channel(64);
        let connection = Arc::new(Connection {
            outgoing,
            pending: Mutex::default(),
        });
        self.connection.send_replace(Some(Arc::clone(&connection)));
        let mut requests = HashMap::new();
        let _ = relay(upgraded, messages, stopped, |message| {
            self.receive(&connection, &mut requests, message)
        })
        .await;
        self.connection.send_replace(None);
        connection.pending().clear();
        for request in requests.values() {
            request.abort();
        }
    }

    /// Passes the plugin's `message` to the delivery it answers, or runs the
    /// call it carries, sending its results on `connection`.
    fn receive(
        &self,
        connection: &Connection,
        requests: &mut HashMap<u64, AbortHandle>,
        message: &[u8],
    ) -> io::Result<()> {
        let (message, payload) = native_protocol::decode(message)?;
        let id = message.id;
        let plugin = message.plugin.as_deref().unwrap_or_default();
        let method = message.method.as_deref().unwrap_or_default();
        requests.retain(|_, request| !request.is_finished());
        let task = match message.r#type.as_str() {
            "reply" => {
                let reply =
                    std::str::from_utf8(payload).map_err(|_| "Invalid event reply encoding");
                connection.answer(id, reply);
                return Ok(());
            }
            "error" => {
                connection.answer(
                    id,
                    Err(std::str::from_utf8(payload).unwrap_or("Invalid event error encoding")),
                );
                return Ok(());
            }
            "call" => {
                let result = self.plugins.call(plugin, method, payload);
                let outgoing = connection.outgoing.clone();
                tokio::spawn(async move {
                    send_result(&outgoing, id, result.await).await;
                })
            }
            "subscribe" => {
                let mut subscription = self.plugins.subscribe(plugin, method, payload);
                let outgoing = connection.outgoing.clone();
                tokio::spawn(async move {
                    while let Some(result) = subscription.next().await {
                        if !send_result(&outgoing, id, result).await {
                            return;
                        }
                    }
                })
            }
            "cancel" => {
                if let Some(request) = requests.remove(&id) {
                    request.abort();
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        if let Some(previous) = requests.insert(id, task.abort_handle()) {
            previous.abort();
        }
        Ok(())
    }
}

/// Sends the plugin's encoded result for the call or subscription,
/// returning whether the dev WebSocket is still open.
async fn send_result(
    outgoing: &mpsc::Sender<(Header, Vec<u8>)>,
    id: u64,
    result: PluginReply,
) -> bool {
    outgoing
        .send((
            Header {
                r#type: "result".into(),
                id,
                failed: result.failed,
                done: result.done,
                ..Header::default()
            },
            result.payload,
        ))
        .await
        .is_ok()
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
    mut messages: mpsc::Receiver<(Header, Vec<u8>)>,
    stopped: &CancellationToken,
    mut receive: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(TokioIo::new(upgraded));
    let mut buffer = Vec::new();
    let mut bytes = [0; 8192];
    // The text of a message whose frames have not all arrived.
    let mut message = Vec::new();
    loop {
        while let Some(frame) = parse_websocket_frame(&mut buffer, false)? {
            match frame.opcode {
                0x0 | 0x2 => message.extend_from_slice(&frame.payload),
                0x8 => return Ok(()),
                0x9 => write_frame(&mut writer, 0xA, &frame.payload).await?,
                _ => continue,
            }
            if frame.final_frame && matches!(frame.opcode, 0x0 | 0x2) {
                receive(&std::mem::take(&mut message))?;
            }
        }
        tokio::select! {
            () = stopped.cancelled() => return Ok(()),
            message = messages.recv() => match message {
                Some((header, payload)) => write_frame(&mut writer, 0x2, &native_protocol::encode(&header, &payload)?).await?,
                None => return Ok(()),
            },
            count = reader.read(&mut bytes) => match count? {
                0 => return Ok(()),
                count => buffer.extend_from_slice(&bytes[..count]),
            },
        }
    }
}
