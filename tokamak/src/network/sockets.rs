use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::time::Duration;

use futures_util::{
    FutureExt,
    future::{LocalBoxFuture, Shared},
};
use rquickjs::{Ctx, Exception, Function, Object, Promise, TypedArray, function::Async};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::Mutex,
};
use tokio_native_tls::{TlsConnector, TlsStream};
use tokio_util::{either::Either, sync::CancellationToken};

type Connection = Either<TcpStream, TlsStream<TcpStream>>;
type Opening = Shared<LocalBoxFuture<'static, Result<Rc<Socket>, String>>>;

pub(crate) fn ip_version(input: &str) -> u8 {
    let address = match input.split_once('%') {
        Some((address, zone))
            if !zone.is_empty()
                && zone.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b':')
                })
                && address.contains(':') =>
        {
            address
        }
        Some(_) => return 0,
        None => input,
    };
    match address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(_)) => 4,
        Ok(std::net::IpAddr::V6(_)) => 6,
        Err(_) => 0,
    }
}

struct Socket {
    reader: Mutex<Option<ReadHalf<Connection>>>,
    writer: Mutex<Option<WriteHalf<Connection>>>,
    closed: CancellationToken,
    upgrading: Cell<bool>,
}

struct UpgradeGuard<'a>(&'a Socket);

impl Drop for UpgradeGuard<'_> {
    fn drop(&mut self) {
        self.0.upgrading.set(false);
        self.0.close();
    }
}

impl Socket {
    fn new(connection: Connection, closed: CancellationToken) -> Rc<Self> {
        let (reader, writer) = tokio::io::split(connection);
        Rc::new(Self {
            reader: Mutex::new(Some(reader)),
            writer: Mutex::new(Some(writer)),
            closed,
            upgrading: Cell::new(false),
        })
    }

    fn close(&self) {
        self.closed.cancel();
        if let Ok(mut reader) = self.reader.try_lock() {
            reader.take();
        }
        if let Ok(mut writer) = self.writer.try_lock() {
            writer.take();
        }
    }

    async fn read(&self) -> io::Result<Option<Vec<u8>>> {
        let mut reader = self.reader.lock().await;
        let Some(stream) = reader.as_mut() else {
            return Ok(None);
        };
        let mut bytes = vec![0; 16384];
        let count = tokio::select! {
            biased;
            () = self.closed.cancelled() => Err(closed_error()),
            count = stream.read(&mut bytes) => count,
        };
        let count = match count {
            Ok(count) => count,
            Err(error) => {
                if !self.upgrading.get() {
                    self.close();
                    reader.take();
                }
                return Err(error);
            }
        };
        if count == 0 {
            return Ok(None);
        }
        bytes.truncate(count);
        Ok(Some(bytes))
    }

    async fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.writer.lock().await;
        let stream = writer.as_mut().ok_or_else(closed_error)?;
        let result = tokio::select! {
            biased;
            () = self.closed.cancelled() => Err(closed_error()),
            result = stream.write_all(bytes) => result,
        };
        if result.is_err() && !self.upgrading.get() {
            self.close();
            writer.take();
        }
        result
    }

    async fn shutdown(&self) -> io::Result<()> {
        let mut writer = self.writer.lock().await;
        let Some(stream) = writer.as_mut() else {
            return Ok(());
        };
        tokio::select! {
            biased;
            () = self.closed.cancelled() => return Err(closed_error()),
            result = stream.shutdown() => result?,
        }
        writer.take();
        Ok(())
    }

    async fn start_tls(&self, host: &str) -> io::Result<Connection> {
        self.upgrading.set(true);
        let _guard = UpgradeGuard(self);
        self.closed.cancel();
        let reader = self.reader.lock().await.take().ok_or_else(closed_error)?;
        let writer = self.writer.lock().await.take().ok_or_else(closed_error)?;
        match reader.unsplit(writer) {
            Either::Left(stream) => tls(stream, host).await.map(Either::Right),
            Either::Right(_) => Err(io::Error::other("Socket is already secure")),
        }
    }
}

async fn tls(stream: TcpStream, host: &str) -> io::Result<TlsStream<TcpStream>> {
    let connector = native_tls::TlsConnector::new().map_err(io::Error::other)?;
    TlsConnector::from(connector)
        .connect(host, stream)
        .await
        .map_err(io::Error::other)
}

async fn connect(host: &str, port: u16, secure: bool) -> io::Result<Connection> {
    let stream = TcpStream::connect((host, port)).await?;
    if secure {
        tls(stream, host).await.map(Either::Right)
    } else {
        Ok(Either::Left(stream))
    }
}

fn opening(
    operation: impl std::future::Future<Output = io::Result<Connection>> + 'static,
    cancelled: CancellationToken,
) -> Opening {
    async move {
        let connection = tokio::select! {
            biased;
            () = cancelled.cancelled() => return Err("Socket is closed".to_owned()),
            result = tokio::time::timeout(Duration::from_secs(15), operation) => result.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?,
        };
        Ok(Socket::new(connection, cancelled))
    }.boxed_local().shared()
}

pub(crate) fn start(
    ctx: Ctx<'_>,
    host: String,
    port: u16,
    secure: bool,
) -> rquickjs::Result<Object<'_>> {
    let cancelled = CancellationToken::new();
    let pending = opening(
        async move { connect(&host, port, secure).await },
        cancelled.clone(),
    );
    socket_object(ctx, pending, cancelled)
}

fn socket_object<'js>(
    ctx: Ctx<'js>,
    pending: Opening,
    cancelled: CancellationToken,
) -> rquickjs::Result<Object<'js>> {
    let result = Object::new(ctx.clone())?;
    let ready = pending.clone();
    let future_ctx = ctx.clone();
    result.set(
        "opened",
        Promise::wrap_future(&ctx, async move {
            ready
                .await
                .map(|_| ())
                .map_err(|error| failure(&future_ctx, error))
        })?,
    )?;
    let closing = pending.clone();
    result.set(
        "close",
        Function::new(ctx.clone(), move || {
            cancelled.cancel();
            if let Some(Ok(socket)) = closing.peek() {
                socket.close();
            }
        })?,
    )?;
    result.set("read", read_function(ctx.clone(), pending.clone())?)?;
    result.set("write", write_function(ctx.clone(), pending.clone())?)?;
    let shutdown = pending.clone();
    result.set(
        "shutdown",
        Function::new(
            ctx.clone(),
            Async(move |ctx: Ctx<'js>| {
                let pending = shutdown.clone();
                async move {
                    let socket = pending.await.map_err(|error| failure(&ctx, error))?;
                    socket
                        .shutdown()
                        .await
                        .map_err(|error| failure(&ctx, error))
                }
            }),
        )?,
    )?;
    result.set(
        "startTls",
        Function::new(ctx, move |ctx: Ctx<'js>, host: String| {
            let previous = pending.clone();
            let cancelled = CancellationToken::new();
            let upgraded = opening(
                async move {
                    let socket = previous.await.map_err(io::Error::other)?;
                    socket.start_tls(&host).await
                },
                cancelled.clone(),
            );
            socket_object(ctx, upgraded, cancelled)
        })?,
    )?;
    Ok(result)
}

fn read_function<'js>(ctx: Ctx<'js>, pending: Opening) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx,
        Async(move |ctx: Ctx<'js>| {
            let pending = pending.clone();
            async move {
                let socket = pending.await.map_err(|error| failure(&ctx, error))?;
                socket
                    .read()
                    .await
                    .map_err(|error| failure(&ctx, error))?
                    .map(|bytes| TypedArray::new(ctx, bytes))
                    .transpose()
            }
        }),
    )
}

fn write_function<'js>(ctx: Ctx<'js>, pending: Opening) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx,
        Async(move |ctx: Ctx<'js>, bytes: TypedArray<'js, u8>| {
            let pending = pending.clone();
            async move {
                let bytes = bytes
                    .as_bytes()
                    .ok_or_else(|| failure(&ctx, "Detached socket data"))?
                    .to_vec();
                let socket = pending.await.map_err(|error| failure(&ctx, error))?;
                socket
                    .write(&bytes)
                    .await
                    .map_err(|error| failure(&ctx, error))
            }
        }),
    )
}

fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "Socket is closed")
}

fn failure(ctx: &Ctx<'_>, error: impl std::fmt::Display) -> rquickjs::Error {
    Exception::throw_message(ctx, &error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Poll};

    #[tokio::test]
    async fn cancelled_tls_upgrade_releases_the_plaintext_connection() -> io::Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let connection = TcpStream::connect(listener.local_addr()?).await?;
        let (mut peer, _) = listener.accept().await?;
        let socket = Socket::new(Either::Left(connection), CancellationToken::new());
        let mut read = Box::pin(socket.read());
        let mut upgrade = Box::pin(socket.start_tls("localhost"));
        std::future::poll_fn(|cx| {
            assert!(read.as_mut().poll(cx).is_pending());
            assert!(upgrade.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(upgrade);
        assert!(read.await.is_err());
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(100), peer.read(&mut byte)).await??,
            0
        );
        assert!(socket.reader.lock().await.is_none());
        assert!(socket.writer.lock().await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn close_releases_a_connection_with_a_pending_read() -> io::Result<()> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let connection = TcpStream::connect(listener.local_addr()?).await?;
        let (mut peer, _) = listener.accept().await?;
        let socket = Socket::new(Either::Left(connection), CancellationToken::new());
        let (read, ()) = tokio::join!(socket.read(), async {
            tokio::task::yield_now().await;
            socket.close();
        });
        assert!(read.is_err());
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(100), peer.read(&mut byte)).await??,
            0
        );
        assert!(socket.reader.lock().await.is_none());
        assert!(socket.writer.lock().await.is_none());
        Ok(())
    }
}
