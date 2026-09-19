use flume::{Receiver, SendError, Sender};
use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rustls::ServerConfig;
use tokio::runtime::{Builder as TokioBuilder, Runtime as TokioRuntime};

use crate::lifecycle_events::{Event, Events};
use crate::quickjs::Error;
use crate::readiness::{Readiness, Waker};

use crate::transport::{
    HttpRequest, HttpResponse, ResponseOutcome, TlsStream, is_connect, is_websocket,
    read_header_block, read_request, tls_accept, tls_close, websocket_session,
    write_plain_response, write_response,
};

const MAX_WEBSOCKET_QUEUE: usize = 100;
/// How long an idle persistent connection waits for its next request.
const KEEP_ALIVE_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Supplies the TLS configuration the gateway accepts connections with.
pub(super) type ServerTls = Arc<dyn Fn() -> Result<Arc<ServerConfig>, Error> + Send + Sync>;

#[derive(Clone)]
pub(super) struct GatewayConfig {
    pub(super) tls: ServerTls,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) require_client_certificate: bool,
}

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
    pub(super) config: GatewayConfig,
    pub(super) tokio: tokio::runtime::Handle,
    pub(super) port: AtomicU16,
    pub(super) accepting: Arc<AtomicBool>,
    pub(super) lifecycle: Lifecycle,
    pub(super) connections: Mutex<Vec<Arc<Connection>>>,
    pub(super) events: Events,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LifecyclePhase {
    Running,
    Suspending,
    Suspended,
    Stopping,
}

struct LifecycleStatus {
    phase: LifecyclePhase,
    active: usize,
}

pub(super) struct Lifecycle {
    status: Mutex<LifecycleStatus>,
    changed: Condvar,
    stopped: tokio::sync::Notify,
}

pub(super) struct Execution<'a> {
    lifecycle: &'a Lifecycle,
    accepting: &'a Arc<AtomicBool>,
}

pub(super) trait Handler: Send + Sync {
    fn handle(&self, job: Job, execution: &Execution<'_>) -> Result<(), Error>;
}

impl Lifecycle {
    pub(super) fn new() -> Self {
        Self {
            status: Mutex::new(LifecycleStatus {
                phase: LifecyclePhase::Running,
                active: 0,
            }),
            changed: Condvar::new(),
            stopped: tokio::sync::Notify::new(),
        }
    }

    pub(super) fn suspend(&self) {
        let mut status = lock_status(&self.status);
        if status.phase == LifecyclePhase::Running {
            status.phase = if status.active == 0 {
                LifecyclePhase::Suspended
            } else {
                LifecyclePhase::Suspending
            };
        }
        self.changed.notify_all();
        self.stopped.notify_waiters();
    }

    pub(super) fn resume(&self) {
        let mut status = lock_status(&self.status);
        if status.phase != LifecyclePhase::Stopping {
            status.phase = LifecyclePhase::Running;
            self.changed.notify_all();
        }
    }

    pub(super) fn stop(&self) {
        let mut status = lock_status(&self.status);
        status.phase = LifecyclePhase::Stopping;
        self.changed.notify_all();
        self.stopped.notify_waiters();
    }

    pub(super) fn enter<'a>(&'a self, accepting: &'a Arc<AtomicBool>) -> Option<Execution<'a>> {
        let mut status = self.wait_for_running(lock_status(&self.status), accepting)?;
        status.active += 1;
        Some(Execution {
            lifecycle: self,
            accepting,
        })
    }

    fn wait_until_running(&self, accepting: &AtomicBool) -> bool {
        self.wait_for_running(lock_status(&self.status), accepting)
            .is_some()
    }

    fn wait_for_running<'a>(
        &self,
        mut status: MutexGuard<'a, LifecycleStatus>,
        accepting: &AtomicBool,
    ) -> Option<MutexGuard<'a, LifecycleStatus>> {
        if !accepting.load(Ordering::Acquire) {
            return None;
        }
        while status.phase != LifecyclePhase::Running {
            if status.phase == LifecyclePhase::Stopping || !accepting.load(Ordering::Acquire) {
                return None;
            }
            status = wait_for_change(&self.changed, status);
        }
        accepting.load(Ordering::Acquire).then_some(status)
    }
}

impl Execution<'_> {
    pub(super) async fn paused(&self) {
        loop {
            let changed = self.lifecycle.stopped.notified();
            if !self.is_running() {
                return;
            }
            changed.await;
        }
    }

    pub(super) async fn cancelled(&self) {
        loop {
            let stopped = self.lifecycle.stopped.notified();
            if !self.accepting.load(Ordering::Acquire)
                || lock_status(&self.lifecycle.status).phase == LifecyclePhase::Stopping
            {
                return;
            }
            stopped.await;
        }
    }

    pub(super) fn is_running(&self) -> bool {
        self.accepting.load(Ordering::Acquire)
            && lock_status(&self.lifecycle.status).phase == LifecyclePhase::Running
    }

    /// An owned handle on the accepting flag for observers that outlive this execution's borrow.
    pub(super) fn accepting(&self) -> Arc<AtomicBool> {
        Arc::clone(self.accepting)
    }
}

impl Drop for Execution<'_> {
    fn drop(&mut self) {
        let mut status = lock_status(&self.lifecycle.status);
        status.active -= 1;
        if status.phase == LifecyclePhase::Suspending && status.active == 0 {
            status.phase = LifecyclePhase::Suspended;
        }
        self.lifecycle.changed.notify_all();
    }
}

fn lock_status(status: &Mutex<LifecycleStatus>) -> MutexGuard<'_, LifecycleStatus> {
    status.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wait_for_change<'a>(
    changed: &Condvar,
    status: MutexGuard<'a, LifecycleStatus>,
) -> MutexGuard<'a, LifecycleStatus> {
    changed.wait(status).unwrap_or_else(PoisonError::into_inner)
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
    pub(crate) fn start(
        handler: Arc<dyn Handler>,
        config: GatewayConfig,
        events: Events,
    ) -> Result<Self, Error> {
        let listener = TcpListener::bind(("127.0.0.1", config.port))?;
        let port = listener.local_addr()?.port();
        let tokio_runtime = TokioBuilder::new_multi_thread()
            .thread_name("tokamak-tokio")
            .enable_all()
            .build()
            .map_err(|error| Error::Startup(format!("failed to start Tokio: {error}")))?;
        let shared = Arc::new(Shared {
            handler,
            config,
            tokio: tokio_runtime.handle().clone(),
            port: AtomicU16::new(port),
            accepting: Arc::new(AtomicBool::new(true)),
            lifecycle: Lifecycle::new(),
            connections: Mutex::new(Vec::new()),
            events,
        });
        let gateway_shared = Arc::clone(&shared);
        let gateway = thread::Builder::new()
            .name("tokamak-gateway".to_owned())
            .spawn(move || gateway_loop(&gateway_shared, listener))
            .map_err(|error| Error::Startup(format!("failed to start gateway thread: {error}")))?;
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

    pub(crate) fn suspend(&self) {
        self.shared.lifecycle.suspend();
        close_connections(&self.shared);
    }

    pub(crate) fn resume(&self) {
        self.shared.lifecycle.resume();
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shared.lifecycle.stop();
        self.shared.accepting.store(false, Ordering::Release);
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
    let mut listener_error_reported = false;
    loop {
        reap_finished_connections(&mut connection_threads);
        if !shared.accepting.load(Ordering::Acquire) {
            break;
        }
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if listener_was_closed(&error) => {
                let port = shared.port.load(Ordering::Acquire);
                eprintln!("gateway listener closed: {error}");
                match replace_closed_listener(listener, port) {
                    Ok((replacement, replacement_port)) => {
                        listener = replacement;
                        shared.port.store(replacement_port, Ordering::Release);
                        listener_error_reported = false;
                        eprintln!("tokamak gateway listening on 127.0.0.1:{replacement_port}");
                        continue;
                    }
                    Err(error) => {
                        eprintln!("gateway listener could not recover: {error}");
                        break;
                    }
                }
            }
            Err(error) => {
                if !listener_error_reported {
                    eprintln!("gateway listener failed: {error}");
                    listener_error_reported = true;
                }
                continue;
            }
        };
        listener_error_reported = false;
        if !shared.lifecycle.wait_until_running(&shared.accepting) {
            break;
        }
        let connection_shared = Arc::clone(shared);
        if let Ok(thread) = thread::Builder::new()
            .name("tokamak-connection".to_owned())
            .spawn(move || {
                if let Err(error) = serve_connection(&connection_shared, stream) {
                    connection_shared.events.emit(Event::RequestFailed {
                        message: error.to_string(),
                    });
                }
            })
        {
            connection_threads.push(thread);
        }
    }
    close_connections(shared);
    for thread in connection_threads {
        let _ = thread.join();
    }
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

pub(super) fn serve_connection(shared: &Arc<Shared>, mut stream: TcpStream) -> Result<(), Error> {
    let connection = Arc::new(Connection {
        stream: stream.try_clone()?,
        cancelled: AtomicBool::new(false),
    });
    lock_connections(shared).push(Arc::clone(&connection));
    let _guard = ConnectionGuard {
        shared: Arc::clone(shared),
        connection: Arc::clone(&connection),
    };
    if !shared.accepting.load(Ordering::Acquire) {
        return Ok(());
    }
    if !shared.lifecycle.wait_until_running(&shared.accepting) {
        return Ok(());
    }
    let connect = read_header_block(&mut stream, "HTTP headers")?;
    if !is_connect(&connect, &shared.config.host) {
        write_plain_response(&mut stream, HttpResponse::text(400, "Bad CONNECT request"))?;
        return Ok(());
    }
    stream.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    stream.flush()?;

    let mut tls = tls_accept((shared.config.tls)()?, stream)?;
    if shared.config.require_client_certificate && tls.conn.peer_certificates().is_none() {
        write_response(
            &mut tls,
            HttpResponse::text(403, "Client certificate required"),
            "GET",
            &connection.cancelled,
            false,
        )?;
        return tls_close(&mut tls);
    }
    loop {
        tls.get_ref()
            .set_read_timeout(Some(KEEP_ALIVE_IDLE_TIMEOUT))?;
        let Some(request) = read_request(&mut tls, &shared.config.host)? else {
            return finish_connection(&mut tls, &connection);
        };
        let persistent = request.persistent;
        let method = request.method.clone();
        let websocket_key = request
            .headers
            .get("sec-websocket-key")
            .map(|value| value.to_str().map(str::to_owned))
            .transpose()
            .map_err(io::Error::other)?;
        let (response, websocket_bridge) = dispatch(shared, request)?;
        let response = match response {
            JobResponse::WebSocket => {
                return websocket_session(
                    &mut tls,
                    websocket_key.as_deref(),
                    websocket_bridge.ok_or_else(|| {
                        Error::Startup("WebSocket bridge was not created".to_owned())
                    })?,
                );
            }
            JobResponse::Http(response) => response,
        };
        let outcome = write_response(
            &mut tls,
            response,
            &method,
            &connection.cancelled,
            persistent,
        )?;
        if outcome == ResponseOutcome::Interrupted {
            return Ok(());
        }
        if !persistent {
            return finish_connection(&mut tls, &connection);
        }
    }
}

/// Hands a request to the JavaScript side and waits for its response.
fn dispatch(
    shared: &Arc<Shared>,
    request: HttpRequest,
) -> Result<(JobResponse, Option<WebSocketBridge>), Error> {
    let (websocket_job, websocket_bridge) = if is_websocket(&request) {
        let (job, bridge) = websocket_channels()?;
        (Some(job), Some(bridge))
    } else {
        (None, None)
    };
    let (response, result) = flume::bounded(1);
    let job = Job {
        request,
        response,
        websocket: websocket_job,
    };
    let execution_shared = Arc::clone(shared);
    drop(
        shared
            .tokio
            .spawn_blocking(move || execute_job(&execution_shared, job)),
    );
    let response = result
        .recv()
        .map_err(|_| Error::Startup("JavaScript request was dropped".to_owned()))?;
    Ok((response, websocket_bridge))
}

/// Closes the TLS session unless the runtime already shut the connection down.
fn finish_connection(tls: &mut TlsStream, connection: &Connection) -> Result<(), Error> {
    if connection.cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    tls_close(tls)
}

pub(super) fn websocket_channels() -> Result<(WebSocketJob, WebSocketBridge), Error> {
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
    let Some(execution) = shared.lifecycle.enter(&shared.accepting) else {
        return;
    };
    let response = job.response.clone();
    if let Err(error) = shared.handler.handle(job, &execution) {
        let message = format!("Worker error: {error}");
        let _ = response.send(JobResponse::Http(HttpResponse::text(500, &message)));
        shared.events.emit(Event::RequestFailed { message });
    }
}
