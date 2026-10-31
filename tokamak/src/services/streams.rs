use std::sync::Arc;

use rquickjs::{Ctx, Function, Object, Promise};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{Message, Reply};

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

    pub(super) fn take<'js>(
        self: &Arc<Self>,
        stream: Object<'js>,
        bootstrap: &Object<'js>,
    ) -> rquickjs::Result<Port> {
        let source: Object = bootstrap
            .get::<_, Function>("openServiceStream")?
            .call((stream,))?;
        let (sender, receiver) = flume::bounded(1);
        let streams = Arc::clone(self);
        let bootstrap = bootstrap.clone();
        let token = self.tasks.token();
        source.ctx().clone().spawn(async move {
            let _token = token;
            serve(source, receiver, &bootstrap, &streams).await;
        });
        Ok(Port {
            commands: sender,
            closed: self.closed.clone(),
        })
    }

    pub(super) fn receive<'js>(
        self: &Arc<Self>,
        ctx: &Ctx<'js>,
        port: Port,
        bootstrap: &Object<'js>,
    ) -> rquickjs::Result<Object<'js>> {
        let port = Arc::new(port);
        let abandoned = Arc::downgrade(&port);
        let closed = self.closed.clone();
        ctx.spawn(async move {
            closed.cancelled().await;
            if let Some(port) = abandoned.upgrade() {
                let (reply, _) = flume::bounded(1);
                let _ = port
                    .commands
                    .send_async(Command::Cancel(Message::undefined(), reply))
                    .await;
            }
        });
        let reading = Arc::clone(&port);
        let streams = Arc::clone(self);
        let read = Function::new(ctx.clone(), move |ctx: Ctx<'js>, bootstrap: Object<'js>| {
            let port = Arc::clone(&reading);
            let streams = Arc::clone(&streams);
            let calling = ctx.clone();
            Promise::wrap_future(&ctx, async move {
                request(&port, Command::Read)
                    .await
                    .map_err(|error| crate::errors::throw_message(&calling, &error))?
                    .into_js(&calling, &bootstrap, &streams)
            })
        })?;
        let streams = Arc::clone(self);
        let cancel = Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, reason: Object<'js>, bootstrap: Object<'js>| {
                let reason = Message::from_js(&reason, &bootstrap, &streams)?;
                let port = Arc::clone(&port);
                let streams = Arc::clone(&streams);
                let calling = ctx.clone();
                Promise::wrap_future(&ctx, async move {
                    request(&port, |reply| Command::Cancel(reason, reply))
                        .await
                        .map_err(|error| crate::errors::throw_message(&calling, &error))?
                        .into_js(&calling, &bootstrap, &streams)
                })
            },
        )?;
        bootstrap
            .get::<_, Function>("receiveServiceStream")?
            .call((read, cancel))
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
    source: Object<'js>,
    requests: flume::Receiver<Command>,
    bootstrap: &Object<'js>,
    streams: &Arc<Streams>,
) {
    loop {
        let command = next_command(&requests, streams).await;
        let Some(Command::Read(reply)) = command else {
            return end_stream(&source, command, bootstrap, streams).await;
        };
        if !read(&source, &requests, reply, bootstrap, streams).await {
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
    source: &Object<'js>,
    requests: &flume::Receiver<Command>,
    reply: Response,
    bootstrap: &Object<'js>,
    streams: &Arc<Streams>,
) -> bool {
    let result = async {
        let promise: Promise = source.get::<_, Function>("read")?.call(())?;
        promise.into_future::<Object>().await
    };
    let result = tokio::select! {
        result = result => result,
        command = requests.recv_async() => {
            end_stream(source, command.ok(), bootstrap, streams).await;
            let _ = reply.send(Err("Stream was cancelled".into()));
            return false;
        }
        () = streams.closed.cancelled() => {
            end_stream(source, None, bootstrap, streams).await;
            let _ = reply.send(Err("Stream invocation ended".into()));
            return false;
        }
    };
    let done = result
        .as_ref()
        .map_or(true, |result| result.get("done").unwrap_or(true));
    let result = result.and_then(|result| Reply::from_js(&result, bootstrap, streams));
    let _ = reply.send(result.map_err(|error| error.to_string()));
    !done
}

/// Answers the command that ends the stream: a cancel with the source's
/// cancel result and a read with an error. With no command, as once the port
/// is gone or the streams close, the source is cancelled.
async fn end_stream<'js>(
    source: &Object<'js>,
    command: Option<Command>,
    bootstrap: &Object<'js>,
    streams: &Arc<Streams>,
) {
    match command {
        Some(Command::Cancel(reason, reply)) => {
            let _ = reply.send(cancel(source, reason, bootstrap, streams).await);
        }
        Some(Command::Read(reply)) => {
            let _ = reply.send(Err("Concurrent stream reads are not supported".into()));
        }
        None => {
            let _ = cancel(source, Message::undefined(), bootstrap, streams).await;
        }
    }
}

async fn cancel<'js>(
    source: &Object<'js>,
    reason: Message,
    bootstrap: &Object<'js>,
    streams: &Arc<Streams>,
) -> Result<Reply, String> {
    let cancel = async {
        let reason = reason.into_js(source.ctx(), bootstrap, streams)?;
        let promise: Promise = source.get::<_, Function>("cancel")?.call((reason,))?;
        let result = promise.into_future::<Object>().await?;
        Reply::from_js(&result, bootstrap, streams)
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
