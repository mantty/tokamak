use flume::{Receiver, SendError, Sender};
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hyper::header::HeaderMap;
use tokio::runtime::{Builder as TokioBuilder, Runtime as TokioRuntime};
use tokio_util::sync::CancellationToken;

use crate::certificates::Certificates;
use crate::lifecycle_events::{Event, Events};
use crate::readiness::{Readiness, Waker};

use crate::transport::{
    HttpRequest, HttpResponse, ResponseOutcome, TlsStream, append_header, is_connect, is_websocket,
    read_header_block, read_request, tls_accept, tls_close, websocket_session,
    write_plain_response, write_response,
};

const MAX_WEBSOCKET_QUEUE: usize = 100;
/// How long an idle persistent connection waits for its next request.
const KEEP_ALIVE_IDLE_TIMEOUT: Duration = Duration::from_mins(1);

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

pub(super) struct Shared {
    pub(super) handler: Arc<dyn Handler>,
    pub(super) certificates: Arc<Certificates>,
    pub(super) host: String,
    pub(super) tokio: tokio::runtime::Handle,
    pub(super) port: AtomicU16,
    /// Cancelled once the runtime stops.
    pub(super) stopped: CancellationToken,
    pub(super) connections: Mutex<Vec<Arc<Connection>>>,
    pub(super) events: Events,
}

pub(super) trait Handler: Send + Sync {
    /// Answers `job`, abandoning it once `stopped` is cancelled.
    fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError>;

    /// Delivers `event` to the Worker's listeners, sending what they did to
    /// `event.delivered` by its deadline. Their invocation may run on, for
    /// listeners it registered on plugins, until `stopped` is cancelled.
    fn deliver(&self, event: EventJob, stopped: &CancellationToken);

    /// Starts work that lasts until `stopped` is cancelled.
    fn start(&self, _tokio: &tokio::runtime::Handle, _stopped: &CancellationToken) {}
}

/// An event for the Worker's listeners.
pub(crate) struct EventJob {
    pub(crate) name: String,
    /// The event's JSON.
    pub(crate) event: String,
    pub(crate) deadline: Instant,
    /// Receives what the Worker did with the event, or why it failed.
    pub(crate) delivered: Sender<Result<Delivery, HandlerError>>,
}

/// What the Worker did with an event.
pub(crate) struct Delivery {
    /// The reply, as JSON.
    pub(crate) reply: String,
    /// The events that have listeners, when the Worker reports them.
    pub(crate) listened: Option<BTreeSet<String>>,
}

pub(super) struct Job {
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
pub(super) struct WebSocketOutgoing {
    // `sender` drops before `waker`, so the final wake finds the channel disconnected.
    sender: Sender<WebSocketOutbound>,
    waker: Waker,
}

impl WebSocketOutgoing {
    pub(super) fn send(
        &self,
        frame: WebSocketOutbound,
    ) -> Result<(), SendError<WebSocketOutbound>> {
        self.sender.send(frame)?;
        self.wake();
        Ok(())
    }

    pub(super) async fn send_async(
        &self,
        frame: WebSocketOutbound,
    ) -> Result<(), SendError<WebSocketOutbound>> {
        self.sender.send_async(frame).await?;
        self.wake();
        Ok(())
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
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let tokio_runtime = TokioBuilder::new_multi_thread()
            .thread_name("tokamak-tokio")
            .enable_all()
            .build()?;
        let shared = Arc::new(Shared {
            handler,
            certificates,
            host,
            tokio: tokio_runtime.handle().clone(),
            port: AtomicU16::new(port),
            stopped: CancellationToken::new(),
            connections: Mutex::new(Vec::new()),
            events,
        });
        shared.handler.start(&shared.tokio, &shared.stopped);
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
        self.shared.port.load(Ordering::Acquire)
    }

    pub(crate) fn restore_gateway(&self) -> io::Result<u16> {
        wait_for_gateway(|| self.port())
    }

    /// The state its handler's events are delivered through.
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

/// Delivers the event `name`, whose JSON is `event`, to the handler on a
/// blocking Tokio thread, as it does a request. The handler returns by
/// `deadline`.
pub(crate) fn deliver(
    shared: &Arc<Shared>,
    name: &str,
    event: &str,
    deadline: Instant,
) -> Result<Delivery, String> {
    let (delivered, result) = flume::bounded(1);
    let job = EventJob {
        name: name.to_owned(),
        event: event.to_owned(),
        deadline,
        delivered,
    };
    let worker = Arc::clone(shared);
    drop(shared.tokio.spawn_blocking(move || {
        if !worker.stopped.is_cancelled() {
            worker.handler.deliver(job, &worker.stopped);
        }
    }));
    match result.recv() {
        Ok(delivery) => delivery.map_err(|error| error.to_string()),
        Err(_) => Err(format!("the runtime stopped before {name} was delivered")),
    }
}

/// What the app serves at `path`, which the handler answers as it does the
/// `WebView`'s requests, by `deadline`.
pub(crate) fn fetch(
    shared: &Arc<Shared>,
    path: &str,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
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
    let response = match spawn_job(shared, request, None).recv_deadline(deadline) {
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

fn gateway_loop(shared: &Arc<Shared>, mut listener: TcpListener) {
    let mut connection_threads = Vec::new();
    let mut listener_failed = false;
    loop {
        reap_finished_connections(&mut connection_threads);
        if shared.stopped.is_cancelled() {
            break;
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if listener_was_closed(&error) => {
                let Some(replacement) = recover_listener(shared, listener) else {
                    break;
                };
                listener = replacement;
                continue;
            }
            Err(error) => {
                if !listener_failed {
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

/// A listener replacing the closed `listener`, reported as listening, or
/// `None` once no port can be bound.
fn recover_listener(shared: &Shared, listener: TcpListener) -> Option<TcpListener> {
    match replace_closed_listener(listener, shared.port.load(Ordering::Acquire)) {
        Ok((replacement, port)) => {
            shared.port.store(port, Ordering::Release);
            shared.events.emit(Event::Listening { port });
            Some(replacement)
        }
        Err(error) => {
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
            if let Err(error) = serve_connection(&shared, stream) {
                shared.events.emit(Event::RequestFailed {
                    message: error.to_string(),
                });
            }
        })
}

#[cfg(unix)]
pub(super) fn listener_was_closed(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::EBADF)
}

#[cfg(not(unix))]
pub(super) fn listener_was_closed(_: &io::Error) -> bool {
    false
}

fn replace_closed_listener(listener: TcpListener, port: u16) -> io::Result<(TcpListener, u16)> {
    std::mem::forget(listener);
    bind_replacement_listener(port)
}

pub(super) fn bind_replacement_listener(port: u16) -> io::Result<(TcpListener, u16)> {
    let replacement = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(port_error) => TcpListener::bind(("127.0.0.1", 0)).map_err(|random_error| {
            io::Error::new(
                random_error.kind(),
                format!(
                    "port {port} is unavailable ({port_error}); random port failed: {random_error}"
                ),
            )
        })?,
    };
    let replacement_port = replacement.local_addr()?.port();
    Ok((replacement, replacement_port))
}

pub(super) struct Connection {
    stream: TcpStream,
    cancelled: AtomicBool,
}

struct ConnectionGuard {
    shared: Arc<Shared>,
    connection: Arc<Connection>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        lock_connections(&self.shared)
            .retain(|connection| !Arc::ptr_eq(connection, &self.connection));
    }
}

pub(super) fn lock_connections(shared: &Shared) -> MutexGuard<'_, Vec<Arc<Connection>>> {
    shared
        .connections
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn close_connections(shared: &Shared) {
    for connection in lock_connections(shared).iter() {
        connection.cancelled.store(true, Ordering::Release);
        let _ = connection.stream.shutdown(Shutdown::Both);
    }
}

pub(super) fn probe_gateway(port: u16) -> io::Result<()> {
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

pub(super) fn wait_for_gateway(mut port: impl FnMut() -> u16) -> io::Result<u16> {
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

pub(super) fn serve_connection(shared: &Arc<Shared>, stream: TcpStream) -> io::Result<()> {
    let connection = Arc::new(Connection {
        stream: stream.try_clone()?,
        cancelled: AtomicBool::new(false),
    });
    lock_connections(shared).push(Arc::clone(&connection));
    let _guard = ConnectionGuard {
        shared: Arc::clone(shared),
        connection: Arc::clone(&connection),
    };
    if shared.stopped.is_cancelled() {
        return Ok(());
    }
    let Some(mut tls) = open_tunnel(shared, stream, &connection)? else {
        return Ok(());
    };
    serve_requests(shared, &mut tls, &connection)
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
        let response = match dispatch(shared, request)? {
            (JobResponse::WebSocket, Some(bridge)) => {
                return websocket_session(tls, websocket_key.as_deref(), bridge);
            }
            (JobResponse::WebSocket, None) => {
                return Err(io::Error::other(
                    "a plain request was answered as a WebSocket",
                ));
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
) -> io::Result<(JobResponse, Option<WebSocketBridge>)> {
    let (websocket_job, websocket_bridge) = is_websocket(&request)
        .then(websocket_channels)
        .transpose()?
        .unzip();
    let result = spawn_job(shared, request, websocket_job);
    let response = result.recv().map_err(|_| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "the request ended without a response",
        )
    })?;
    Ok((response, websocket_bridge))
}

/// Runs `request` on a blocking Tokio thread and returns where its response arrives.
fn spawn_job(
    shared: &Arc<Shared>,
    request: HttpRequest,
    websocket: Option<WebSocketJob>,
) -> Receiver<JobResponse> {
    let (response, result) = flume::bounded(1);
    let job = Job {
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

pub(super) fn execute_job(shared: &Shared, job: Job) {
    if shared.stopped.is_cancelled() {
        return;
    }
    let response = job.response.clone();
    if let Err(error) = shared.handler.handle(job, &shared.stopped) {
        let message = format!("Worker error: {error}");
        let _ = response.send(JobResponse::Http(HttpResponse::text(500, &message)));
        shared.events.emit(Event::RequestFailed { message });
    }
}
