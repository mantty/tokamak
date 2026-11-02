use std::cell::Cell;
use std::io;
use std::rc::Rc;
use std::time::Duration;
use tokamak_macros::host_function;

use futures_util::{
    FutureExt,
    future::{LocalBoxFuture, Shared},
};
use rquickjs::{Class, Ctx, JsLifetime, TypedArray, class::Trace};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::Mutex,
};
use tokio_rustls::{TlsConnector, client::TlsStream};
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
    let Ok(address) = address.parse::<std::net::IpAddr>() else {
        return 0;
    };
    match address {
        std::net::IpAddr::V4(_) => 4,
        std::net::IpAddr::V6(_) => 6,
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
        let Either::Left(stream) = reader.unsplit(writer) else {
            return Err(io::Error::other("Socket is already secure"));
        };
        tls(stream, host).await.map(Either::Right)
    }
}

async fn tls(stream: TcpStream, host: &str) -> io::Result<TlsStream<TcpStream>> {
    let config = crate::tls::client_config().map_err(io::Error::other)?;
    let name = rustls_pki_types::ServerName::try_from(host.to_owned()).map_err(io::Error::other)?;
    TlsConnector::from(config).connect(name, stream).await
}

async fn connect(host: &str, port: u16, secure: bool) -> io::Result<Connection> {
    let stream = TcpStream::connect((host, port)).await?;
    if !secure {
        return Ok(Either::Left(stream));
    }
    tls(stream, host).await.map(Either::Right)
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

#[host_function]
pub(crate) fn start(
    ctx: Ctx<'_>,
    host: String,
    port: u16,
    secure: bool,
) -> rquickjs::Result<Class<'_, SocketConnection>> {
    let cancelled = CancellationToken::new();
    let pending = opening(
        async move { connect(&host, port, secure).await },
        cancelled.clone(),
    );
    Class::instance(ctx, SocketConnection { pending, cancelled })
}

/// A socket's connection, usable once `opened` resolves.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct SocketConnection {
    #[qjs(skip_trace)]
    pending: Opening,
    #[qjs(skip_trace)]
    cancelled: CancellationToken,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods(rename_all = "camelCase")]
impl SocketConnection {
    pub async fn opened(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.socket(&ctx).await.map(|_| ())
    }

    pub fn close(&self) {
        self.cancelled.cancel();
        if let Some(Ok(socket)) = self.pending.peek() {
            socket.close();
        }
    }

    /// The next bytes read, or `undefined` once the peer ends the stream.
    pub async fn read<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Option<TypedArray<'js, u8>>> {
        let socket = self.socket(&ctx).await?;
        socket
            .read()
            .await
            .map_err(|error| failure(&ctx, error))?
            .map(|bytes| TypedArray::new(ctx, bytes))
            .transpose()
    }

    pub async fn write<'js>(
        &self,
        ctx: Ctx<'js>,
        bytes: TypedArray<'js, u8>,
    ) -> rquickjs::Result<()> {
        let bytes = bytes
            .as_bytes()
            .ok_or_else(|| failure(&ctx, "Detached socket data"))?
            .to_vec();
        let socket = self.socket(&ctx).await?;
        socket
            .write(&bytes)
            .await
            .map_err(|error| failure(&ctx, error))
    }

    pub async fn shutdown(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        let socket = self.socket(&ctx).await?;
        socket
            .shutdown()
            .await
            .map_err(|error| failure(&ctx, error))
    }

    /// A connection over this one, secured by TLS for `host`.
    pub fn start_tls<'js>(
        &self,
        ctx: Ctx<'js>,
        host: String,
    ) -> rquickjs::Result<Class<'js, Self>> {
        let previous = self.pending.clone();
        let cancelled = CancellationToken::new();
        let pending = opening(
            async move {
                let socket = previous.await.map_err(io::Error::other)?;
                socket.start_tls(&host).await
            },
            cancelled.clone(),
        );
        Class::instance(ctx, Self { pending, cancelled })
    }
}

impl SocketConnection {
    async fn socket(&self, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<Socket>> {
        self.pending
            .clone()
            .await
            .map_err(|error| failure(ctx, error))
    }
}

fn closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "Socket is closed")
}

fn failure(ctx: &Ctx<'_>, error: impl std::fmt::Display) -> rquickjs::Error {
    crate::errors::throw_message(ctx, &error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Poll};

    /// A socket over a connected loopback pair, with the peer's end.
    async fn plaintext_socket() -> io::Result<(Rc<Socket>, TcpStream)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let connection = TcpStream::connect(listener.local_addr()?).await?;
        let (peer, _) = listener.accept().await?;
        let socket = Socket::new(Either::Left(connection), CancellationToken::new());
        Ok((socket, peer))
    }

    /// Assert that the socket released its connection, which `peer` sees close.
    async fn assert_released(socket: &Socket, peer: &mut TcpStream) -> io::Result<()> {
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
    async fn cancelled_tls_upgrade_releases_the_plaintext_connection() -> io::Result<()> {
        let (socket, mut peer) = plaintext_socket().await?;
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
        assert_released(&socket, &mut peer).await?;
        Ok(())
    }

    #[tokio::test]
    async fn close_releases_a_connection_with_a_pending_read() -> io::Result<()> {
        let (socket, mut peer) = plaintext_socket().await?;
        let (read, ()) = tokio::join!(socket.read(), async {
            tokio::task::yield_now().await;
            socket.close();
        });
        assert!(read.is_err());
        assert_released(&socket, &mut peer).await?;
        Ok(())
    }
}
