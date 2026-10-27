use flume::{Receiver, Sender};
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hyper::header::HeaderMap;
use tokio::runtime::{Builder as TokioBuilder, Runtime as TokioRuntime};
use tokio_util::sync::CancellationToken;

use crate::certificates::Certificates;
use crate::lifecycle_events::{Event, Events};
use crate::lock;
use crate::readiness::{Readiness, Waker};

use crate::transport::{
    HttpRequest, HttpResponse, ResponseOutcome, TlsStream, append_header, client_disconnected,
    connection_ended, is_connect, is_websocket, read_header_block, read_request, tls_accept,
    tls_close, websocket_session, write_plain_response, write_response,
};

const MAX_WEBSOCKET_QUEUE: usize = 100;
/// How long an idle persistent connection waits for its next request.
const KEEP_ALIVE_IDLE_TIMEOUT: Duration = Duration::from_mins(1);
/// How long the gateway waits after an accept error before accepting again.
const ACCEPT_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Why a handler could not answer a request.
pub(super) type HandlerError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) struct Runtime {
    shared: Arc<Shared>,
    gateway: Option<JoinHandle<()>>,
    tokio: Option<TokioRuntime>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GatewayRuntime")
            .field("port", &self.port())
            .finish_non_exhaustive()
    }
}

struct Shared {
    handler: Arc<dyn Handler>,
    certificates: Arc<Certificates>,
    host: String,
    tokio: tokio::runtime::Handle,
    port: Port,
    /// Cancelled once the runtime stops. The handler gets child tokens of it,
    /// which see the gateway stop but cannot stop it.
    stopped: CancellationToken,
    connections: Mutex<Vec<Arc<Connection>>>,
    events: Events,
}

/// The port the gateway listens on, which a shell waits on to learn that it
/// moved, or 0 once the gateway lost its port and could not bind another.
struct Port {
    current: Mutex<u16>,
    changed: Condvar,
}

impl Port {
    fn new(port: u16) -> Self {
        Self {
            current: Mutex::new(port),
            changed: Condvar::new(),
        }
    }

    fn get(&self) -> u16 {
        *lock(&self.current)
    }

    fn set(&self, port: u16) {
        *lock(&self.current) = port;
        self.changed.notify_all();
    }

    /// The port once it differs from `known`, or after `timeout` the current port.
    fn wait_for_change(&self, known: u16, timeout: Duration) -> u16 {
        let (current, _) = self
            .changed
            .wait_timeout_while(lock(&self.current), timeout, |port| *port == known)
            .unwrap_or_else(PoisonError::into_inner);
        *current
    }
}

pub(super) trait Handler: Send + Sync {
    /// Answers `job`, abandoning it once `stopped` is cancelled.
    fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError>;

    /// Delivers `event`, completing its invocation by the event's deadline.
    fn deliver(&self, event: EventJob, stopped: &CancellationToken);

    /// Starts work that lasts until `stopped` is cancelled.
    fn start(&self, _tokio: &tokio::runtime::Handle, _stopped: &CancellationToken) {}
}

/// An event for the Worker's listeners.
pub(crate) struct EventJob {
    pub(crate) name: String,
    /// The encoded event payload.
    pub(crate) event: Vec<u8>,
    pub(crate) deadline: Instant,
    /// Receives what the Worker did with the event, or why it failed.
    pub(crate) delivered: Sender<Result<Delivery, HandlerError>>,
}

/// What the Worker did with an event.
pub(crate) struct Delivery {
    /// The encoded reply payload.
    pub(crate) reply: Vec<u8>,
    /// The events that have listeners, when the Worker reports them.
    pub(crate) listened: Option<BTreeSet<String>>,
}

pub(super) struct Job {
    pub(super) cancelled: CancellationToken,
    pub(super) deadline: Option<Instant>,
    pub(super) request: HttpRequest,
    pub(super) response: Sender<JobResponse>,
    pub(super) websocket: Option<WebSocketJob>,
}

pub(super) enum JobResponse {
    Http(HttpResponse),
    WebSocket,
}

pub(super) struct WebSocketJob {
    pub(super) incoming: Receiver<WebSocketInbound>,
    pub(super) outgoing: WebSocketOutgoing,
}

pub(super) struct WebSocketBridge {
    pub(super) incoming: Sender<WebSocketInbound>,
    pub(super) outgoing: Receiver<WebSocketOutbound>,
    pub(super) readiness: Readiness,
}

/// Queues frames for the connection thread and wakes its readiness wait.
///
/// Frames sent once the connection has ended are dropped. The end reaches the
/// handler through the job's `incoming` channel.
pub(super) struct WebSocketOutgoing {
    // `sender` drops before `waker`, so the final wake finds the channel disconnected.
    sender: Sender<WebSocketOutbound>,
    waker: Waker,
}

impl WebSocketOutgoing {
    pub(super) fn send(&self, frame: WebSocketOutbound) {
        if self.sender.send(frame).is_ok() {
            self.wake();
        }
    }

    pub(super) async fn send_async(&self, frame: WebSocketOutbound) {
        if self.sender.send_async(frame).await.is_ok() {
            self.wake();
        }
    }

    fn wake(&self) {
        // A failed wake delays the frame until the next socket event; the frame is still queued.
        let _ = self.waker.wake();
    }
}

pub(super) enum WebSocketInbound {
    Message { binary: bool, payload: Vec<u8> },
    Close { code: u16, reason: String },
}

pub(super) enum WebSocketOutbound {
    Message { binary: bool, payload: Vec<u8> },
    Close { code: u16, reason: String },
    Ready,
}

impl Runtime {
    /// Serves `handler` on a loopback port, terminating TLS for `host` with
    /// `certificates`.
    pub(crate) fn start(
        handler: Arc<dyn Handler>,
        certificates: Arc<Certificates>,
        host: String,
        events: Events,
    ) -> io::Result<Self> {
        let listener = Listener::bind(0)?;
        let port = listener.port;
        let tokio_runtime = TokioBuilder::new_multi_thread()
            .thread_name("tokamak-tokio")
            .enable_all()
            .build()?;
        let shared = Arc::new(Shared {
            handler,
            certificates,
            host,
            tokio: tokio_runtime.handle().clone(),
            port: Port::new(port),
            stopped: CancellationToken::new(),
            connections: Mutex::new(Vec::new()),
            events,
        });
        shared
            .handler
            .start(&shared.tokio, &shared.stopped.child_token());
        let gateway_shared = Arc::clone(&shared);
        let gateway = thread::Builder::new()
            .name("tokamak-gateway".to_owned())
            .spawn(move || gateway_loop(&gateway_shared, listener))?;
        shared.events.emit(Event::Listening { port });
        Ok(Self {
            shared,
            gateway: Some(gateway),
            tokio: Some(tokio_runtime),
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.shared.port.get()
    }

    pub(crate) fn wait_for_port_change(&self, known: u16, timeout: Duration) -> u16 {
        self.shared.port.wait_for_change(known, timeout)
    }

    pub(crate) fn restore_gateway(&self) -> io::Result<u16> {
        wait_for_gateway(|| self.port())
    }

    /// Delivers events and fetches paths through the gateway's handler.
    pub(crate) fn handle(&self) -> Handle {
        Handle(Arc::clone(&self.shared))
    }
}

/// Delivers events and fetches paths through a running gateway's handler,
/// without being able to stop the gateway or change its port.
pub(crate) struct Handle(Arc<Shared>);

impl Handle {
    /// Delivers the event `name`, whose JSON is `event`, to the handler on a
    /// blocking Tokio thread, as it does a request. The handler returns by
    /// `deadline`.
    pub(crate) fn deliver(
        &self,
        name: &str,
        event: &[u8],
        deadline: Instant,
    ) -> Result<Delivery, String> {
        let (delivered, result) = flume::bounded(1);
        let job = EventJob {
            name: name.to_owned(),
            event: event.to_owned(),
            deadline,
            delivered,
        };
        let worker = Arc::clone(&self.0);
        drop(self.0.tokio.spawn_blocking(move || {
            if !worker.stopped.is_cancelled() {
                worker.handler.deliver(job, &worker.stopped.child_token());
            }
        }));
        match result.recv() {
            Ok(delivery) => delivery.map_err(|error| error.to_string()),
            Err(_) => Err(format!("the runtime stopped before {name} was delivered")),
        }
    }

    /// What the app serves at `path`, which the handler answers as it does
    /// the `WebView`'s requests, by `deadline`.
    pub(crate) fn fetch(&self, path: &str, deadline: Instant) -> Result<Vec<u8>, String> {
        let shared = &self.0;
        if !path.starts_with('/') {
            return Err(format!("{path} is not a path the app serves"));
        }
        let mut headers = HeaderMap::new();
        append_header(&mut headers, "host", &shared.host).map_err(|error| error.to_string())?;
        let request = HttpRequest {
            persistent: false,
            method: "GET".to_owned(),
            target: path.to_owned(),
            url: format!("https://{}{path}", shared.host),
            headers,
            body: None,
        };
        let cancelled = shared.stopped.child_token();
        let _request = cancelled.clone().drop_guard();
        let response = match spawn_job(shared, request, None, cancelled, Some(deadline))
            .recv_deadline(deadline)
        {
            Ok(JobResponse::Http(response)) => response,
            Ok(JobResponse::WebSocket) => return Err(format!("GET {path} answered a WebSocket")),
            Err(_) => return Err(format!("GET {path} timed out")),
        };
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "GET {path} answered {} {}",
                response.status, response.status_text
            ));
        }
        response.body.collect(deadline)
    }

    /// Reports `message` as a failure, unless the gateway has stopped.
    pub(crate) fn report_failure(&self, message: String) {
        if !self.0.stopped.is_cancelled() {
            self.0.events.emit(Event::Failed { message });
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shared.stopped.cancel();
        close_connections(&self.shared);
        let _ = TcpStream::connect(("127.0.0.1", self.port()));
        if let Some(thread) = self.gateway.take() {
            let _ = thread.join();
        }
        if let Some(tokio) = self.tokio.take() {
            tokio.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

fn gateway_loop(shared: &Arc<Shared>, mut listener: Listener) {
    let mut connection_threads = Vec::new();
    let mut listener_failed = false;
    loop {
        reap_finished_connections(&mut connection_threads);
        if shared.stopped.is_cancelled() {
            break;
        }
        let stream = match listener.socket.accept() {
            Ok((stream, _)) => stream,
            Err(error) if listener_was_lost(&listener, &error) => {
                let Some(replacement) = replace_lost_listener(shared, listener) else {
                    break;
                };
                listener = replacement;
                continue;
            }
            Err(error) => {
                if listener_failed {
                    thread::sleep(ACCEPT_RETRY_DELAY);
                } else {
                    shared.events.emit(Event::Failed {
                        message: format!("gateway listener failed: {error}"),
                    });
                    listener_failed = true;
                }
                continue;
            }
        };
        listener_failed = false;
        if shared.stopped.is_cancelled() {
            break;
        }
        if let Ok(thread) = spawn_connection(shared, stream) {
            connection_threads.push(thread);
        }
    }
    close_connections(shared);
    for thread in connection_threads {
        let _ = thread.join();
    }
}

/// The replacement listener, whose port the gateway reports as listening, or
/// `None` with port 0 reported once no port could be bound.
fn publish_replacement(shared: &Shared, replacement: io::Result<Listener>) -> Option<Listener> {
    match replacement {
        Ok(replacement) => {
            let port = replacement.port;
            shared.port.set(port);
            shared.events.emit(Event::Listening { port });
            Some(replacement)
        }
        Err(error) => {
            shared.port.set(0);
            shared.events.emit(Event::Failed {
                message: format!("gateway listener could not recover: {error}"),
            });
            None
        }
    }
}

fn spawn_connection(shared: &Arc<Shared>, stream: TcpStream) -> io::Result<JoinHandle<()>> {
    let shared = Arc::clone(shared);
    thread::Builder::new()
        .name("tokamak-connection".to_owned())
        .spawn(move || {
            // A client that disconnects has not failed.
            if let Err(error) = serve_connection(&shared, stream)
                && !connection_ended(&error)
            {
                shared.events.emit(Event::RequestFailed {
                    message: error.to_string(),
                });
            }
        })
}

/// The gateway's loopback listener and the port it holds.
struct Listener {
    socket: TcpListener,
    port: u16,
    /// A second socket on `port` that never listens. Linux lets sockets share
    /// a port through `SO_REUSEPORT` only when one user owns them all, and
    /// each Android app runs as its own user, so no other app can bind `port`
    /// while this is open, even after `socket` is lost.
    #[cfg(any(target_os = "android", target_os = "linux"))]
    _reservation: socket2::Socket,
}

impl Listener {
    /// A listener on loopback `port`, or on a free port for 0.
    fn bind(port: u16) -> io::Result<Self> {
        let socket = bind_loopback(port)?;
        let port = socket.local_addr()?.port();
        Ok(Self {
            socket,
            port,
            #[cfg(any(target_os = "android", target_os = "linux"))]
            _reservation: user_port_socket(port)?,
        })
    }
}

/// Whether `listener` stopped listening, after `accept` failed. Linux
/// `accept` also reports the network errors of the connection it was
/// accepting, so the socket itself is asked.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn listener_was_lost(listener: &Listener, _: &io::Error) -> bool {
    !socket2::SockRef::from(&listener.socket)
        .is_listener()
        .unwrap_or(false)
}

/// Whether `listener` stopped listening, which `accept` reports with these
/// errors once its descriptor no longer holds a listening stream socket.
#[cfg(all(unix, not(any(target_os = "android", target_os = "linux"))))]
fn listener_was_lost(_: &Listener, error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EBADF | libc::EINVAL | libc::ENOTSOCK | libc::EOPNOTSUPP)
    )
}

/// Whether `listener` stopped listening, which `accept` reports with these
/// errors once its handle no longer holds a listening stream socket.
#[cfg(windows)]
fn listener_was_lost(_: &Listener, error: &io::Error) -> bool {
    use windows_sys::Win32::Networking::WinSock::{WSAEINVAL, WSAENOTSOCK, WSAEOPNOTSUPP};

    matches!(
        error.raw_os_error(),
        Some(WSAENOTSOCK | WSAEINVAL | WSAEOPNOTSUPP)
    )
}

/// A loopback listener on `port`, or on a free port for 0, which no other
/// socket can bind while it is open.
#[cfg(windows)]
fn bind_loopback(port: u16) -> io::Result<TcpListener> {
    use socket2::{Domain, Socket, Type};
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{SO_EXCLUSIVEADDRUSE, SOL_SOCKET, setsockopt};

    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    let raw = usize::try_from(socket.as_raw_socket()).map_err(io::Error::other)?;
    let enabled = 1_i32.to_ne_bytes();
    let length = i32::try_from(enabled.len()).map_err(io::Error::other)?;
    // SAFETY: `raw` is an open socket and `enabled` is `length` readable bytes.
    let result = unsafe {
        setsockopt(
            raw,
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            enabled.as_ptr(),
            length,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    socket.bind(&SocketAddr::from(([127, 0, 0, 1], port)).into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

/// A loopback listener on `port`, or on a free port for 0, which only
/// sockets of this user can share.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn bind_loopback(port: u16) -> io::Result<TcpListener> {
    let socket = user_port_socket(port)?;
    socket.listen(128)?;
    Ok(socket.into())
}

/// A socket bound to loopback `port` with `SO_REUSEPORT`, which Linux lets
/// only sockets of the same user share. It leaves out `SO_REUSEADDR`, which
/// lets any user bind a port that no socket listens on.
#[cfg(any(target_os = "android", target_os = "linux"))]
fn user_port_socket(port: u16) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)?;
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from(([127, 0, 0, 1], port)).into())?;
    Ok(socket)
}

/// A loopback listener on `port`, or on a free port for 0.
#[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
fn bind_loopback(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

/// The listener replacing `lost`, whose descriptor may have been closed and
/// its number reused for another file, published by `publish_replacement`.
/// The lost listener's reservation keeps the port until the replacement holds
/// it.
fn replace_lost_listener(shared: &Shared, lost: Listener) -> Option<Listener> {
    std::mem::forget(lost.socket);
    let replacement = Listener::bind(lost.port)
        .or_else(|error| handle_unavailable_port(shared, lost.port, &error));
    publish_replacement(shared, replacement)
}

/// Ends the process once the gateway cannot bind its port again: the `WebView`
/// trusts a certificate whose key ships in the APK, so whatever holds the port
/// could pose as the gateway.
#[cfg(target_os = "android")]
fn handle_unavailable_port(shared: &Shared, port: u16, error: &io::Error) -> io::Result<Listener> {
    shared.events.emit(Event::Failed {
        message: format!(
            "gateway listener could not recover: port {port} is unavailable ({error})"
        ),
    });
    std::process::abort()
}

/// A listener on a free port once the gateway cannot bind `port` again.
#[cfg(not(target_os = "android"))]
fn handle_unavailable_port(_: &Shared, port: u16, error: &io::Error) -> io::Result<Listener> {
    Listener::bind(0).map_err(|random_error| {
        io::Error::new(
            random_error.kind(),
            format!("port {port} is unavailable ({error}); random port failed: {random_error}"),
        )
    })
}

struct Connection {
    stream: TcpStream,
    cancelled: AtomicBool,
}

struct ConnectionGuard {
    shared: Arc<Shared>,
    connection: Arc<Connection>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        lock(&self.shared.connections)
            .retain(|connection| !Arc::ptr_eq(connection, &self.connection));
    }
}

fn close_connections(shared: &Shared) {
    for connection in lock(&shared.connections).iter() {
        connection.cancelled.store(true, Ordering::Release);
        let _ = connection.stream.shutdown(Shutdown::Both);
        #[cfg(windows)]
        cancel_pending_io(&connection.stream);
    }
}

/// Ends the reads and writes other threads are blocked in on `stream`'s
/// socket, which a Windows `shutdown` leaves waiting for the peer.
#[cfg(windows)]
fn cancel_pending_io(stream: &TcpStream) {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::IO::CancelIoEx;

    let Ok(socket) = usize::try_from(stream.as_raw_socket()) else {
        return;
    };
    // SAFETY: `stream` keeps the socket open, and `CancelIoEx` only cancels
    // the operations pending on it.
    unsafe { CancelIoEx(socket as HANDLE, std::ptr::null()) };
}

fn probe_gateway(port: u16) -> io::Result<()> {
    let timeout = Duration::from_millis(100);
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).map_err(describe("TCP connect failed"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(describe("setting read timeout failed"))?;
    stream
        .write_all(b"CONNECT tokamak-probe.invalid:443 HTTP/1.1\r\n\r\n")
        .map_err(describe("CONNECT write failed"))?;
    let response = io::read_to_string(stream).map_err(describe("CONNECT read failed"))?;
    if response.starts_with("HTTP/1.1 400") && response.ends_with("\r\n\r\nBad CONNECT request") {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "unexpected CONNECT response: {}",
            response.lines().next().unwrap_or("empty response")
        )))
    }
}

fn describe(context: &'static str) -> impl FnOnce(io::Error) -> io::Error {
    move |error| io::Error::new(error.kind(), format!("{context}: {error}"))
}

fn wait_for_gateway(mut port: impl FnMut() -> u16) -> io::Result<u16> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let current = port();
        match probe_gateway(current) {
            Ok(()) => return Ok(current),
            Err(error) if Instant::now() >= deadline => {
                return Err(io::Error::other(format!(
                    "gateway did not recover: {error}"
                )));
            }
            Err(_) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn reap_finished_connections(connections: &mut Vec<JoinHandle<()>>) {
    for thread in connections.extract_if(.., |thread| thread.is_finished()) {
        let _ = thread.join();
    }
}

fn serve_connection(shared: &Arc<Shared>, stream: TcpStream) -> io::Result<()> {
    let connection = Arc::new(Connection {
        stream: stream.try_clone()?,
        cancelled: AtomicBool::new(false),
    });
    lock(&shared.connections).push(Arc::clone(&connection));
    let _guard = ConnectionGuard {
        shared: Arc::clone(shared),
        connection: Arc::clone(&connection),
    };
    if shared.stopped.is_cancelled() {
        return Ok(());
    }
    let served = match open_tunnel(shared, stream, &connection) {
        Ok(Some(mut tls)) => serve_requests(shared, &mut tls, &connection),
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    // A connection the gateway closed has not failed.
    if connection.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    served
}

/// The mutually authenticated TLS session a client opens through a CONNECT
/// tunnel, or `None` once the client was refused.
fn open_tunnel(
    shared: &Shared,
    mut stream: TcpStream,
    connection: &Connection,
) -> io::Result<Option<TlsStream>> {
    let connect = read_header_block(&mut stream, "HTTP headers")?;
    if !is_connect(&connect, &shared.host) {
        write_plain_response(&mut stream, HttpResponse::text(400, "Bad CONNECT request"))?;
        return Ok(None);
    }
    stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    stream.flush()?;
    let tls_config = shared
        .certificates
        .server_config()
        .map_err(io::Error::other)?;
    let mut tls = tls_accept(tls_config, stream)?;
    if tls.conn.peer_certificates().is_some() {
        return Ok(Some(tls));
    }
    write_response(
        &mut tls,
        HttpResponse::text(403, "Client certificate required"),
        "GET",
        &connection.cancelled,
        false,
    )?;
    tls_close(&mut tls)?;
    Ok(None)
}

/// Answers the requests a TLS session carries until it closes or upgrades to
/// a WebSocket.
fn serve_requests(
    shared: &Arc<Shared>,
    tls: &mut TlsStream,
    connection: &Connection,
) -> io::Result<()> {
    loop {
        tls.get_ref()
            .set_read_timeout(Some(KEEP_ALIVE_IDLE_TIMEOUT))?;
        let Some(request) = read_request(tls, &shared.host)? else {
            return finish_connection(tls, connection);
        };
        let persistent = request.persistent;
        let method = request.method.clone();
        let websocket_key = request
            .headers
            .get("sec-websocket-key")
            .map(|value| value.to_str().map(str::to_owned))
            .transpose()
            .map_err(io::Error::other)?;
        let cancelled = shared.stopped.child_token();
        let _request = cancelled.clone().drop_guard();
        let response = match dispatch(shared, request, cancelled, tls)? {
            (JobResponse::WebSocket, bridge) => {
                let bridge = bridge.ok_or_else(|| {
                    io::Error::other("a plain request was answered as a WebSocket")
                })?;
                return websocket_session(tls, websocket_key.as_deref(), bridge);
            }
            (JobResponse::Http(response), _) => response,
        };
        let outcome = write_response(tls, response, &method, &connection.cancelled, persistent)?;
        if outcome == ResponseOutcome::Interrupted {
            return Ok(());
        }
        if !persistent {
            return finish_connection(tls, connection);
        }
    }
}

/// Hands a request to its handler and waits for the response.
fn dispatch(
    shared: &Arc<Shared>,
    request: HttpRequest,
    cancelled: CancellationToken,
    tls: &mut TlsStream,
) -> io::Result<(JobResponse, Option<WebSocketBridge>)> {
    let (websocket_job, websocket_bridge) = is_websocket(&request)
        .then(websocket_channels)
        .transpose()?
        .unzip();
    let result = spawn_job(shared, request, websocket_job, cancelled, None);
    let response = wait_for_response(&result, tls)?;
    Ok((response, websocket_bridge))
}

fn wait_for_response(
    result: &Receiver<JobResponse>,
    tls: &mut TlsStream,
) -> io::Result<JobResponse> {
    let (mut readiness, _wake) = Readiness::new()?;
    readiness.observe(tls.get_ref())?;
    let timeout = tls.get_ref().read_timeout()?;
    tls.get_ref()
        .set_read_timeout(Some(Duration::from_millis(1)))?;
    loop {
        match result.recv_timeout(Duration::from_millis(100)) {
            Ok(response) => {
                tls.get_ref().set_read_timeout(timeout)?;
                return Ok(response);
            }
            Err(flume::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("the request ended without a response"));
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
        }
        if client_disconnected(tls, &mut readiness)? {
            return Err(io::ErrorKind::ConnectionAborted.into());
        }
    }
}

/// Runs `request` on a blocking Tokio thread and returns where its response arrives.
fn spawn_job(
    shared: &Arc<Shared>,
    request: HttpRequest,
    websocket: Option<WebSocketJob>,
    cancelled: CancellationToken,
    deadline: Option<Instant>,
) -> Receiver<JobResponse> {
    let (response, result) = flume::bounded(1);
    let job = Job {
        cancelled,
        deadline,
        request,
        response,
        websocket,
    };
    let execution_shared = Arc::clone(shared);
    drop(
        shared
            .tokio
            .spawn_blocking(move || execute_job(&execution_shared, job)),
    );
    result
}

/// Closes the TLS session unless the runtime already shut the connection down.
fn finish_connection(tls: &mut TlsStream, connection: &Connection) -> io::Result<()> {
    if connection.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    tls_close(tls)
}

pub(super) fn websocket_channels() -> io::Result<(WebSocketJob, WebSocketBridge)> {
    let (incoming_sender, incoming_receiver) = flume::bounded(MAX_WEBSOCKET_QUEUE);
    let (outgoing_sender, outgoing_receiver) = flume::bounded(MAX_WEBSOCKET_QUEUE);
    let (readiness, waker) = Readiness::new()?;
    let job = WebSocketJob {
        incoming: incoming_receiver,
        outgoing: WebSocketOutgoing {
            sender: outgoing_sender,
            waker,
        },
    };
    let bridge = WebSocketBridge {
        incoming: incoming_sender,
        outgoing: outgoing_receiver,
        readiness,
    };
    Ok((job, bridge))
}

fn execute_job(shared: &Shared, job: Job) {
    if shared.stopped.is_cancelled() {
        return;
    }
    let response = job.response.clone();
    if let Err(error) = shared.handler.handle(job, &shared.stopped.child_token()) {
        let message = format!("Worker error: {error}");
        let _ = response.send(JobResponse::Http(HttpResponse::text(500, &message)));
        shared.events.emit(Event::RequestFailed { message });
    }
}

#[cfg(test)]
mod tests;
