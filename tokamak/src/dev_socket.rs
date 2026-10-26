//! The dev WebSocket between the development app and the tokamak Vite plugin,
//! through `tok dev`'s relay. It carries the app's events to the development
//! Worker, and the development Worker's plugin calls to the app.

use std::collections::HashMap;
use std::future::Future;
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
use crate::plugin_calls::{Body, Overflow, PluginCalls, PluginReply, error};
use crate::transport::{parse_websocket_frame, write_frame};

/// The development app's end of the dev WebSocket, while it is open.
pub(crate) struct DevSocket {
    connection: watch::Sender<Option<Arc<Connection>>>,
    next_id: AtomicU64,
    plugins: Arc<PluginCalls>,
}

type PendingReplies = HashMap<u64, oneshot::Sender<Result<Vec<u8>, String>>>;

/// One open dev WebSocket.
struct Connection {
    outgoing: mpsc::Sender<(Header, Vec<u8>)>,
    /// Where each delivery waiting for its answer receives it, by message ID.
    pending: Mutex<PendingReplies>,
}

impl DevSocket {
    pub(crate) fn new(plugins: Arc<PluginCalls>) -> Self {
        Self {
            connection: watch::Sender::new(None),
            next_id: AtomicU64::new(0),
            plugins,
        }
    }

    /// Sends the encoded event `name` once the dev WebSocket is open,
    /// and returns the encoded reply the plugin answers with by
    /// `deadline`.
    pub(crate) async fn deliver(
        &self,
        name: &str,
        event: &[u8],
        deadline: Instant,
    ) -> Result<Vec<u8>, HandlerError> {
        let deadline = tokio::time::Instant::from_std(deadline);
        let late = || format!("{name} did not complete before its deadline");
        let unreachable =
            || format!("{name} could not reach the development server before its deadline");
        let closed = || format!("the dev socket closed before {name} completed");
        let mut connections = self.connection.subscribe();
        let connection = tokio::time::timeout_at(deadline, connections.wait_for(Option::is_some))
            .await
            .map_err(|_| unreachable())??
            .clone()
            .ok_or_else(unreachable)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let message = (
            Header {
                r#type: "event".into(),
                id,
                name: Some(name.into()),
                ..Header::default()
            },
            event.to_vec(),
        );
        let (sender, answer) = oneshot::channel();
        connection.pending().insert(id, sender);
        let Ok(Ok(())) = tokio::time::timeout_at(deadline, connection.outgoing.send(message)).await
        else {
            connection.pending().remove(&id);
            return Err(closed().into());
        };
        let answer = tokio::time::timeout_at(deadline, answer).await;
        connection.pending().remove(&id);
        answer
            .map_err(|_| late())?
            .map_err(|_| closed())?
            .map_err(Into::into)
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
        let mut requests = Requests::default();
        let _ = relay(upgraded, messages, stopped, |message| {
            self.receive(&connection, &mut requests, message)
        })
        .await;
        self.connection.send_replace(None);
        connection.pending().clear();
        for request in requests.tasks.values() {
            request.abort();
        }
    }

    /// Passes the plugin's `message` to the delivery it answers, or runs the
    /// call it carries, sending its results on `connection`.
    fn receive(
        &self,
        connection: &Connection,
        requests: &mut Requests,
        message: &[u8],
    ) -> io::Result<()> {
        let (message, payload) = native_protocol::decode(message)?;
        let id = message.id;
        let plugin = message.plugin.as_deref().unwrap_or_default();
        let method = message.method.as_deref().unwrap_or_default();
        requests.tasks.retain(|_, request| !request.is_finished());
        requests.bodies.retain(|_, chunks| !chunks.is_closed());
        let outgoing = connection.outgoing.clone();
        let task = match message.r#type.as_str() {
            "reply" => {
                connection.answer(id, Ok(payload));
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
                tokio::spawn(async move {
                    send_result(&outgoing, id, result.await).await;
                })
            }
            "callWithBody" => {
                let (result, body) = self.plugins.call_with_body(plugin, method, payload);
                let (chunks, received) = mpsc::unbounded_channel();
                requests.bodies.insert(id, chunks);
                tokio::spawn(async move {
                    let result = with_body(result, body, received, &outgoing, id).await;
                    send_result(&outgoing, id, result).await;
                })
            }
            "chunk" => {
                if let Some(chunks) = requests.bodies.get(&id) {
                    let _ = chunks.send(PluginReply {
                        payload: payload.to_vec(),
                        failed: message.failed,
                        done: message.done,
                    });
                }
                return Ok(());
            }
            "subscribe" => {
                let overflow = message
                    .overflow
                    .as_deref()
                    .map_or(Some(Overflow::Error), Overflow::named);
                let mut subscription = overflow
                    .map(|overflow| self.plugins.subscribe(plugin, method, payload, overflow));
                tokio::spawn(async move {
                    let Some(subscription) = &mut subscription else {
                        let unknown = error("TypeError", "Unknown overflow policy");
                        send_result(&outgoing, id, unknown).await;
                        return;
                    };
                    while let Some(result) = subscription.next().await {
                        if !send_result(&outgoing, id, result).await {
                            return;
                        }
                    }
                })
            }
            "cancel" => {
                if let Some(request) = requests.tasks.remove(&id) {
                    request.abort();
                }
                return Ok(());
            }
            _ => return Ok(()),
        };
        if let Some(previous) = requests.tasks.insert(id, task.abort_handle()) {
            previous.abort();
        }
        Ok(())
    }
}

/// The development Worker's calls and subscriptions running on the app.
#[derive(Default)]
struct Requests {
    tasks: HashMap<u64, AbortHandle>,
    /// Where the host's chunks of each call's body go.
    bodies: HashMap<u64, mpsc::UnboundedSender<PluginReply>>,
}

/// The result of a call whose `body` the shell pulls: each pull goes to the
/// host, whose chunk arrives on `chunks`.
async fn with_body(
    result: impl Future<Output = PluginReply>,
    body: Option<Body>,
    mut chunks: mpsc::UnboundedReceiver<PluginReply>,
    outgoing: &mpsc::Sender<(Header, Vec<u8>)>,
    id: u64,
) -> PluginReply {
    let relay = async {
        let Some(mut body) = body else { return };
        while body.pulled().await {
            let pull = Header {
                r#type: "pull".into(),
                id,
                ..Header::default()
            };
            if outgoing.send((pull, Vec::new())).await.is_err() {
                return;
            }
            let Some(chunk) = chunks.recv().await else {
                return;
            };
            body.send(&chunk);
            if chunk.done {
                return;
            }
        }
    };
    tokio::pin!(result);
    tokio::select! {
        result = &mut result => result,
        () = relay => result.await,
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
    fn pending(&self) -> std::sync::MutexGuard<'_, PendingReplies> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Passes the plugin's `answer` to the delivery of the event `id`.
    fn answer(&self, id: u64, answer: Result<&[u8], &str>) {
        if let Some(sender) = self.pending().remove(&id) {
            let _ = sender.send(answer.map(<[u8]>::to_vec).map_err(str::to_owned));
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::DevSocket;
    use crate::plugin_calls::PluginCalls;

    #[tokio::test]
    async fn fails_an_event_while_the_development_server_is_unreachable() {
        let socket = DevSocket::new(PluginCalls::new(None, true));
        let deadline = Instant::now() + Duration::from_millis(50);

        let error = socket.deliver("example.ping", b"{}", deadline).await.err();

        assert_eq!(
            error.map(|error| error.to_string()).as_deref(),
            Some("example.ping could not reach the development server before its deadline")
        );
    }
}
