//! The dev WebSocket, which carries events from the development app to the
//! tokamak Vite plugin through `tok dev`'s relay.

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
use tokio_util::sync::CancellationToken;

use crate::gateway::HandlerError;
use crate::transport::{parse_websocket_frame, write_frame};

/// The development app's end of the dev WebSocket, while it is open.
pub(crate) struct DevSocket {
    connection: watch::Sender<Option<Arc<Connection>>>,
    next_id: AtomicU64,
}

/// One open dev WebSocket.
struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    /// Where each delivery waiting for its answer receives it, by message ID.
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<String, String>>>>,
}

/// A message the app sends.
#[derive(Serialize)]
struct EventMessage<'a> {
    r#type: &'static str,
    id: u64,
    name: &'a str,
    event: &'a RawValue,
}

/// A message the plugin sends: the reply to the event `id`, or why it failed.
#[derive(Deserialize)]
struct Answer<'a> {
    r#type: &'a str,
    id: u64,
    #[serde(borrow)]
    reply: Option<&'a RawValue>,
    message: Option<String>,
}

impl DevSocket {
    pub(crate) fn new() -> Self {
        Self {
            connection: watch::Sender::new(None),
            next_id: AtomicU64::new(0),
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
    /// delivering events through it meanwhile.
    pub(crate) async fn serve(&self, upgraded: Upgraded, stopped: &CancellationToken) {
        let (outgoing, messages) = mpsc::unbounded_channel();
        let connection = Arc::new(Connection {
            outgoing,
            pending: Mutex::default(),
        });
        self.connection.send_replace(Some(Arc::clone(&connection)));
        let _ = relay(upgraded, &connection, messages, stopped).await;
        self.connection.send_replace(None);
        connection.pending().clear();
    }
}

impl Connection {
    fn pending(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u64, oneshot::Sender<Result<String, String>>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Passes the plugin's `message` to the delivery it answers.
    fn answer(&self, message: &[u8]) {
        let Ok(answer) = serde_json::from_slice::<Answer>(message) else {
            return;
        };
        let result = match answer.r#type {
            "reply" => Ok(answer.reply.map_or("null", RawValue::get).to_owned()),
            "error" => Err(answer.message.unwrap_or_default()),
            _ => return,
        };
        if let Some(sender) = self.pending().remove(&answer.id) {
            let _ = sender.send(result);
        }
    }
}

async fn relay(
    upgraded: Upgraded,
    connection: &Connection,
    mut messages: mpsc::UnboundedReceiver<String>,
    stopped: &CancellationToken,
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
                connection.answer(&std::mem::take(&mut message));
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
