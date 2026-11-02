use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, FromJs, JsLifetime};
use tokamak_macros::JsType;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{JsMessage, JsReply, Message, Reply};
use crate::typed::{JsFunction, JsPromise, JsReadableStream};

type Response = flume::Sender<Result<Reply, String>>;

enum Command {
    Read(Response),
    Cancel(Message, Response),
}

#[derive(Debug)]
pub(super) struct Port {
    commands: flume::Sender<Command>,
    closed: CancellationToken,
}

#[derive(Default)]
pub(crate) struct Streams {
    tasks: TaskTracker,
    closed: CancellationToken,
}

impl Streams {
    pub(crate) fn active(&self) -> bool {
        !self.tasks.is_empty()
    }

    pub(crate) async fn until_none(&self) {
        self.tasks.close();
        self.tasks.wait().await;
    }

    pub(crate) fn cancel(&self) {
        self.closed.cancel();
    }
}

/// The runtime's functions that move the streams in service messages.
#[derive(Clone, FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct StreamCodec<'js> {
    /// Takes a stream that a message sends, which the host then reads.
    open_service_stream: JsFunction<'js, (JsReadableStream<'js>,), StreamSource<'js>>,
    /// The stream that reads a stream a message carries through its port.
    receive_service_stream:
        JsFunction<'js, (Class<'js, ServiceStreamPort>,), JsReadableStream<'js>>,
}

/// The runtime's reader of a stream that a message sends.
#[derive(FromJs, JsType)]
pub(crate) struct StreamSource<'js> {
    read: JsFunction<'js, (), JsPromise<'js, StreamRead<'js>>>,
    cancel: JsFunction<'js, (JsMessage<'js>,), JsPromise<'js, JsReply<'js>>>,
}

/// A read of a stream that a message sends: a reply, and whether it ended the
/// stream.
#[derive(FromJs, JsType)]
pub(crate) struct StreamRead<'js> {
    done: bool,
    failed: bool,
    value: JsMessage<'js>,
}

/// Converts service messages to and from one JavaScript context, whose
/// `codec` opens the streams a message sends and receives the streams it
/// carries.
#[derive(Clone)]
pub(crate) struct MessageCodec<'js> {
    ctx: Ctx<'js>,
    codec: StreamCodec<'js>,
    streams: Arc<Streams>,
}

impl<'js> MessageCodec<'js> {
    pub(crate) fn new(ctx: &Ctx<'js>, codec: StreamCodec<'js>, streams: &Arc<Streams>) -> Self {
        Self {
            ctx: ctx.clone(),
            codec,
            streams: Arc::clone(streams),
        }
    }

    pub(super) fn ctx(&self) -> &Ctx<'js> {
        &self.ctx
    }

    pub(super) fn take(&self, stream: JsReadableStream<'js>) -> rquickjs::Result<Port> {
        let source = self.codec.open_service_stream.call((stream,))?;
        let (sender, receiver) = flume::bounded(1);
        let codec = self.clone();
        let token = self.streams.tasks.token();
        self.ctx.spawn(async move {
            let _token = token;
            serve(source, receiver, &codec).await;
        });
        Ok(Port {
            commands: sender,
            closed: self.streams.closed.clone(),
        })
    }

    pub(super) fn receive(&self, port: Port) -> rquickjs::Result<JsReadableStream<'js>> {
        let port = Arc::new(port);
        let abandoned = Arc::downgrade(&port);
        let closed = self.streams.closed.clone();
        self.ctx.spawn(async move {
            closed.cancelled().await;
            if let Some(port) = abandoned.upgrade() {
                let (reply, _) = flume::bounded(1);
                let _ = port
                    .commands
                    .send_async(Command::Cancel(Message::undefined(), reply))
                    .await;
            }
        });
        let port = ServiceStreamPort {
            port,
            streams: Arc::clone(&self.streams),
        };
        self.codec
            .receive_service_stream
            .call((Class::instance(self.ctx.clone(), port)?,))
    }
}

/// The port of a stream that a message carries, which the runtime reads and
/// cancels through the stream's producer.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct ServiceStreamPort {
    #[qjs(skip_trace)]
    port: Arc<Port>,
    #[qjs(skip_trace)]
    streams: Arc<Streams>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl ServiceStreamPort {
    /// The producer's next read, whose streams `codec` moves.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn read<'js>(
        &self,
        ctx: Ctx<'js>,
        codec: StreamCodec<'js>,
    ) -> rquickjs::Result<JsPromise<'js, JsReply<'js>>> {
        let port = Arc::clone(&self.port);
        let codec = MessageCodec::new(&ctx, codec, &self.streams);
        let calling = ctx.clone();
        JsPromise::wrap_future(&ctx, async move {
            request(&port, Command::Read)
                .await
                .map_err(|error| crate::errors::throw_message(&calling, &error))?
                .into_js(&codec)
        })
    }

    /// Cancels the stream with `reason`, whose streams `codec` moves.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn cancel<'js>(
        &self,
        ctx: Ctx<'js>,
        reason: JsMessage<'js>,
        codec: StreamCodec<'js>,
    ) -> rquickjs::Result<JsPromise<'js, JsReply<'js>>> {
        let codec = MessageCodec::new(&ctx, codec, &self.streams);
        let reason = Message::from_js(reason, &codec)?;
        let port = Arc::clone(&self.port);
        let calling = ctx.clone();
        JsPromise::wrap_future(&ctx, async move {
            request(&port, |reply| Command::Cancel(reason, reply))
                .await
                .map_err(|error| crate::errors::throw_message(&calling, &error))?
                .into_js(&codec)
        })
    }
}

async fn request(port: &Port, command: impl FnOnce(Response) -> Command) -> Result<Reply, String> {
    let (sender, receiver) = flume::bounded(1);
    let response = async {
        port.commands
            .send_async(command(sender))
            .await
            .map_err(|_| "Stream producer is no longer available".to_owned())?;
        receiver
            .recv_async()
            .await
            .map_err(|_| "Stream producer is no longer available".to_owned())?
    };
    tokio::select! {
        biased;
        result = response => result,
        () = port.closed.cancelled() => Err("Stream invocation ended".into()),
    }
}

async fn serve<'js>(
    source: StreamSource<'js>,
    requests: flume::Receiver<Command>,
    codec: &MessageCodec<'js>,
) {
    loop {
        let command = next_command(&requests, &codec.streams).await;
        let Some(Command::Read(reply)) = command else {
            return end_stream(&source, command, codec).await;
        };
        if !read(&source, &requests, reply, codec).await {
            return;
        }
    }
}

/// The port's next command, or `None` once the port is gone or the streams
/// close.
async fn next_command(requests: &flume::Receiver<Command>, streams: &Streams) -> Option<Command> {
    tokio::select! {
        command = requests.recv_async() => command.ok(),
        () = streams.closed.cancelled() => None,
    }
}

async fn read<'js>(
    source: &StreamSource<'js>,
    requests: &flume::Receiver<Command>,
    reply: Response,
    codec: &MessageCodec<'js>,
) -> bool {
    let result = async { source.read.call(())?.into_future().await };
    let result = tokio::select! {
        result = result => result,
        command = requests.recv_async() => {
            end_stream(source, command.ok(), codec).await;
            let _ = reply.send(Err("Stream was cancelled".into()));
            return false;
        }
        () = codec.streams.closed.cancelled() => {
            end_stream(source, None, codec).await;
            let _ = reply.send(Err("Stream invocation ended".into()));
            return false;
        }
    };
    let done = result.as_ref().map_or(true, |result| result.done);
    let result = result.and_then(|read| {
        let reply = JsReply {
            failed: read.failed,
            value: read.value,
        };
        Reply::from_js(reply, codec)
    });
    let _ = reply.send(result.map_err(|error| error.to_string()));
    !done
}

/// Answers the command that ends the stream: a cancel with the source's
/// cancel result and a read with an error. With no command, as once the port
/// is gone or the streams close, the source is cancelled.
async fn end_stream<'js>(
    source: &StreamSource<'js>,
    command: Option<Command>,
    codec: &MessageCodec<'js>,
) {
    match command {
        Some(Command::Cancel(reason, reply)) => {
            let _ = reply.send(cancel(source, reason, codec).await);
        }
        Some(Command::Read(reply)) => {
            let _ = reply.send(Err("Concurrent stream reads are not supported".into()));
        }
        None => {
            let _ = cancel(source, Message::undefined(), codec).await;
        }
    }
}

async fn cancel<'js>(
    source: &StreamSource<'js>,
    reason: Message,
    codec: &MessageCodec<'js>,
) -> Result<Reply, String> {
    let cancel = async {
        let reason = reason.into_js(codec)?;
        let result = source.cancel.call((reason,))?.into_future().await?;
        Reply::from_js(result, codec)
    };
    cancel
        .await
        .map_err(|error: rquickjs::Error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{Command, Message, Port, Reply, request};
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn a_final_reply_survives_immediate_producer_teardown()
    -> Result<(), Box<dyn std::error::Error>> {
        for failed in [false, true] {
            let (commands, requests) = flume::bounded(1);
            let closed = CancellationToken::new();
            let port = Port {
                commands,
                closed: closed.clone(),
            };
            let producer = reply_and_close(&requests, &closed, failed);
            let (result, completion) = tokio::join!(request(&port, Command::Read), producer);
            completion?;
            assert_eq!(result?.failed, failed);
        }
        Ok(())
    }

    /// Answers the first read on `requests` with a reply of `failed`, then
    /// closes the port at once.
    async fn reply_and_close(
        requests: &flume::Receiver<Command>,
        closed: &CancellationToken,
        failed: bool,
    ) -> Result<(), flume::RecvError> {
        let Command::Read(reply) = requests.recv_async().await? else {
            panic!("expected a read");
        };
        assert!(
            reply
                .send(Ok(Reply {
                    failed,
                    value: Message::undefined()
                }))
                .is_ok()
        );
        closed.cancel();
        Ok(())
    }
}
