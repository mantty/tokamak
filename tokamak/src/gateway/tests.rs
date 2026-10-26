#[cfg(unix)]
use super::gateway_loop;
use super::{
    Delivery, EventJob, Handle, Handler, HandlerError, Job, JobResponse, Listener, Port, Runtime,
    Shared, close_connections, execute_job, listener_was_lost, lock_connections, probe_gateway,
    publish_replacement, replace_lost_listener, serve_connection, wait_for_gateway,
};
use crate::lifecycle_events::{Event, Events};
use crate::tests::{HOST, certificates, request};
use crate::transport::HttpResponse;
use flume::Receiver;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[test]
fn reports_handler_failures_through_the_event_listener()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    struct FailingHandler;
    impl Handler for FailingHandler {
        fn handle(&self, _: Job, _: &CancellationToken) -> Result<(), HandlerError> {
            Err("handler exploded".into())
        }

        fn deliver(&self, _: EventJob, _: &CancellationToken) {}
    }
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let shared = shared(directory.path(), Arc::new(FailingHandler), &tokio, recorder)?;
    let (response, responses) = flume::bounded(1);

    execute_job(
        &shared,
        Job {
            cancelled: CancellationToken::new(),
            deadline: None,
            request: request("GET", "/"),
            response,
            websocket: None,
        },
    );

    let JobResponse::Http(response) = responses.try_recv()? else {
        return Err("expected an HTTP error response".into());
    };
    assert_eq!(response.status, 500);
    let events: Vec<Event> = events.try_iter().collect();
    let [Event::RequestFailed { message }] = events.as_slice() else {
        panic!("unexpected events: {events:?}");
    };
    assert!(
        message.starts_with("Worker error: ") && message.ends_with("handler exploded"),
        "unexpected events: {events:?}"
    );
    Ok(())
}

struct UnreachableHandler;

impl Handler for UnreachableHandler {
    fn handle(&self, _: Job, _: &CancellationToken) -> Result<(), HandlerError> {
        Err("handler must not run".into())
    }

    fn deliver(&self, _: EventJob, _: &CancellationToken) {}
}

#[test]
fn reports_connection_failures_through_the_event_listener()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = tempfile::tempdir()?;
    let (recorder, events) = recorded_events();
    let runtime = Runtime::start(
        Arc::new(UnreachableHandler),
        certificates(directory.path())?,
        HOST.to_owned(),
        recorder,
    )?;
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5))?,
        Event::Listening {
            port: runtime.port()
        }
    );

    // A client that hangs up has not failed, but one that answers the tunnel
    // with something other than TLS fails its connection.
    drop(TcpStream::connect(("127.0.0.1", runtime.port()))?);
    let mut client = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    client
        .write_all(format!("CONNECT {HOST}:443 HTTP/1.1\r\n\r\nnot a TLS handshake").as_bytes())?;

    let event = events.recv_timeout(Duration::from_secs(5))?;
    assert!(
        matches!(&event, Event::RequestFailed { message } if message.contains("corrupt message")),
        "unexpected event: {event:?}"
    );
    let late = events.recv_timeout(Duration::from_millis(200));
    assert!(late.is_err(), "unexpected event: {late:?}");
    Ok(())
}

/// Gateway state for `handler`, which runs on `tokio` and reports to `events`.
fn shared(
    directory: &Path,
    handler: Arc<dyn Handler>,
    tokio: &tokio::runtime::Runtime,
    events: Events,
) -> io::Result<Shared> {
    Ok(Shared {
        handler,
        certificates: certificates(directory)?,
        host: HOST.to_owned(),
        tokio: tokio.handle().clone(),
        port: Port::new(0),
        stopped: CancellationToken::new(),
        connections: Mutex::new(Vec::new()),
        events,
    })
}

/// Events that arrive, in order, on the returned receiver.
fn recorded_events() -> (Events, Receiver<Event>) {
    let (sink, events) = flume::unbounded();
    let recorder = Events::new(move |event| {
        let _ = sink.send(event);
    });
    (recorder, events)
}

#[test]
fn shutdown_closes_registered_connections() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let shared = Arc::new(shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        Events::new(|_| {}),
    )?);
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    let connection_shared = Arc::clone(&shared);
    let (finished, outcome) = flume::bounded(1);
    thread::spawn(move || {
        let _ = finished.send(serve_connection(&connection_shared, server));
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while lock_connections(&shared).is_empty() {
        if Instant::now() > deadline {
            return Err("the connection was never registered".into());
        }
        thread::yield_now();
    }
    close_connections(&shared);
    outcome
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "the connection did not close")??;
    assert!(lock_connections(&shared).is_empty());
    drop(client);
    Ok(())
}

#[test]
#[cfg(not(any(target_os = "android", target_os = "linux")))]
fn recognizes_the_errors_of_a_lost_listener() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(unix)]
    let (lost, failed) = (
        [libc::EBADF, libc::EINVAL, libc::ENOTSOCK, libc::EOPNOTSUPP],
        libc::ECONNABORTED,
    );
    #[cfg(windows)]
    let (lost, failed) = {
        use windows_sys::Win32::Networking::WinSock::{
            WSAECONNRESET, WSAEINVAL, WSAENOTSOCK, WSAEOPNOTSUPP,
        };
        ([WSAENOTSOCK, WSAEINVAL, WSAEOPNOTSUPP], WSAECONNRESET)
    };
    let listener = Listener::bind(0)?;

    for code in lost {
        let error = io::Error::from_raw_os_error(code);
        assert!(listener_was_lost(&listener, &error), "{error}");
    }
    let failed = io::Error::from_raw_os_error(failed);
    assert!(!listener_was_lost(&listener, &failed));
    Ok(())
}

#[test]
fn closed_listener_reclaims_its_port() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let replacement = Listener::bind(port)?;

    assert_eq!(replacement.port, port);
    Ok(())
}

#[test]
#[cfg(not(target_os = "android"))]
fn moves_to_a_free_port_once_its_port_is_unavailable() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let shared = shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        recorder,
    )?;
    let occupied = TcpListener::bind(("127.0.0.1", 0))?;
    let port = occupied.local_addr()?.port();
    // A listener lost while another socket took its port. The lost socket
    // stays open, as `replace_lost_listener` leaves it.
    let lost = Listener {
        port,
        ..Listener::bind(0)?
    };

    let replacement = replace_lost_listener(&shared, lost).ok_or("no replacement")?;

    assert_ne!(replacement.port, port);
    assert_eq!(shared.port.get(), replacement.port);
    assert_eq!(
        events.try_recv()?,
        Event::Listening {
            port: replacement.port
        }
    );
    Ok(())
}

/// Puts the descriptor of `file` in place of `listener`'s, as when a stray
/// close frees the listener's descriptor number and another file takes it.
#[cfg(unix)]
fn reuse_listener_descriptor(listener: &Listener, file: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: both descriptors are open, and `dup2` replaces the listener's.
    if unsafe { libc::dup2(file.as_raw_fd(), listener.socket.as_raw_fd()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn notices_a_listener_whose_descriptor_holds_a_connection() -> Result<(), Box<dyn std::error::Error>>
{
    let listener = Listener::bind(0)?;
    let _client = TcpStream::connect(("127.0.0.1", listener.port))?;
    let (connection, _) = listener.socket.accept()?;
    // The next connection the gateway accepts takes the lowest free number.
    reuse_listener_descriptor(&listener, &connection)?;

    let Err(error) = listener.socket.accept() else {
        return Err("a connection accepted a connection".into());
    };
    assert!(listener_was_lost(&listener, &error));
    Ok(())
}

#[test]
#[cfg(unix)]
fn replaces_a_listener_whose_descriptor_holds_a_file() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let shared = shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        recorder,
    )?;
    let shared = Arc::new(shared);
    let listener = Listener::bind(0)?;
    let port = listener.port;
    let descriptor = listener.socket.as_raw_fd();
    reuse_listener_descriptor(&listener, &std::fs::File::open("/dev/null")?)?;
    let gateway_shared = Arc::clone(&shared);
    let gateway = thread::spawn(move || gateway_loop(&gateway_shared, listener));

    assert_eq!(
        events.recv_timeout(Duration::from_secs(5))?,
        Event::Listening { port }
    );
    wait_for_gateway(|| port)?;
    shared.stopped.cancel();
    drop(TcpStream::connect(("127.0.0.1", port))?);
    gateway.join().map_err(|_| "gateway thread panicked")?;
    // SAFETY: `F_GETFD` only reads the descriptor's flags.
    let open = unsafe { libc::fcntl(descriptor, libc::F_GETFD) } != -1;
    assert!(
        open,
        "the replacement closed the file holding the lost descriptor"
    );
    // SAFETY: the lost listener left the file's descriptor open for the test.
    unsafe { libc::close(descriptor) };
    Ok(())
}

#[test]
#[cfg(any(target_os = "android", target_os = "linux"))]
fn keeps_a_listener_whose_accept_reported_a_network_error() -> Result<(), Box<dyn std::error::Error>>
{
    let listener = Listener::bind(0)?;

    // Linux `accept` reports the network errors of the connection it was
    // accepting, EOPNOTSUPP among them.
    let failed = io::Error::from_raw_os_error(libc::EOPNOTSUPP);
    assert!(!listener_was_lost(&listener, &failed));
    Ok(())
}

#[test]
#[cfg(any(target_os = "android", target_os = "linux"))]
fn reserves_its_port_after_its_listener_closes() -> Result<(), Box<dyn std::error::Error>> {
    let listener = Listener::bind(0)?;
    let port = listener.port;
    drop(listener.socket);

    assert!(
        TcpListener::bind(("127.0.0.1", port)).is_err(),
        "a socket that reuses addresses took the port"
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    assert_eq!(Listener::bind(port)?.port, port);
    Ok(())
}

#[test]
#[cfg(windows)]
fn holds_its_port_against_sockets_that_reuse_addresses() -> Result<(), Box<dyn std::error::Error>> {
    let listener = Listener::bind(0)?;
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None)?;
    socket.set_reuse_address(true)?;

    let address = std::net::SocketAddr::from(([127, 0, 0, 1], listener.port));
    assert!(socket.bind(&address.into()).is_err());
    Ok(())
}

#[test]
fn probes_a_gateway_through_connect() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;
        if !request_line.starts_with("CONNECT tokamak-probe.invalid:443 ") {
            return Err(io::Error::other("unexpected probe request"));
        }
        while reader.read_line(&mut String::new())? > 2 {}
        stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 19\r\n\r\nBad CONNECT request")
    });

    probe_gateway(port)?;
    server.join().map_err(|_| "probe server panicked")??;
    Ok(())
}

#[test]
fn waits_for_the_gateway_to_publish_a_new_port() -> Result<(), Box<dyn std::error::Error>> {
    let old_listener = TcpListener::bind(("127.0.0.1", 0))?;
    let old_port = old_listener.local_addr()?.port();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let new_port = listener.local_addr()?.port();
    drop(old_listener);
    let port = Arc::new(AtomicU16::new(old_port));
    let server_port = Arc::clone(&port);
    let server = thread::spawn(move || -> io::Result<()> {
        thread::sleep(Duration::from_millis(20));
        server_port.store(new_port, Ordering::Release);
        let (mut stream, _) = listener.accept()?;
        let mut reader = BufReader::new(stream.try_clone()?);
        while reader.read_line(&mut String::new())? > 2 {}
        stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 19\r\n\r\nBad CONNECT request")
    });

    assert_eq!(wait_for_gateway(|| port.load(Ordering::Acquire))?, new_port);
    server.join().map_err(|_| "probe server panicked")??;
    Ok(())
}

#[test]
fn reports_an_unexpected_gateway_response() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut reader = BufReader::new(stream.try_clone()?);
        while reader.read_line(&mut String::new())? > 2 {}
        stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
    });

    let error = match probe_gateway(port) {
        Ok(()) => return Err("unexpected response was accepted".into()),
        Err(error) => error,
    };
    assert_eq!(
        error.to_string(),
        "unexpected CONNECT response: HTTP/1.1 503 Service Unavailable"
    );
    server.join().map_err(|_| "probe server panicked")??;
    Ok(())
}

#[test]
fn publishes_the_port_of_a_replacement_listener() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let shared = shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        recorder,
    )?;
    let replacement = Listener::bind(0)?;
    let port = replacement.port;

    assert!(publish_replacement(&shared, Ok(replacement)).is_some());

    assert_eq!(shared.port.get(), port);
    assert_eq!(events.try_recv()?, Event::Listening { port });
    Ok(())
}

#[test]
fn publishes_no_port_once_no_listener_can_be_bound() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let shared = shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        recorder,
    )?;
    shared.port.set(1234);

    let unavailable = Err(io::Error::other("no port is free"));
    assert!(publish_replacement(&shared, unavailable).is_none());

    assert_eq!(shared.port.get(), 0);
    assert!(
        matches!(events.try_recv()?, Event::Failed { message } if message.contains("no port is free"))
    );
    Ok(())
}

#[test]
fn wakes_port_waiters_when_the_port_changes() -> Result<(), Box<dyn std::error::Error>> {
    let port = Arc::new(Port::new(1));
    let recovered = Arc::clone(&port);
    let recovery = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        recovered.set(2);
    });

    let waited = Instant::now();
    assert_eq!(port.wait_for_change(1, Duration::from_secs(5)), 2);
    assert!(
        waited.elapsed() < Duration::from_secs(1),
        "the waiter was not woken"
    );
    assert_eq!(port.wait_for_change(2, Duration::from_millis(10)), 2);
    recovery.join().map_err(|_| "recovery panicked")?;
    Ok(())
}

#[test]
fn keeps_serving_when_its_handler_cancels_the_token_it_was_given()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    struct CancellingHandler;
    impl Handler for CancellingHandler {
        fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError> {
            stopped.cancel();
            let _ = job
                .response
                .send(JobResponse::Http(HttpResponse::text(200, "served")));
            Ok(())
        }

        fn deliver(&self, event: EventJob, stopped: &CancellationToken) {
            stopped.cancel();
            let _ = event.delivered.send(Ok(Delivery {
                reply: Vec::new(),
                listened: None,
            }));
        }

        fn start(&self, _: &tokio::runtime::Handle, stopped: &CancellationToken) {
            stopped.cancel();
        }
    }
    let directory = tempfile::tempdir()?;
    let runtime = Runtime::start(
        Arc::new(CancellingHandler),
        certificates(directory.path())?,
        HOST.to_owned(),
        Events::new(|_| {}),
    )?;
    let handle = runtime.handle();
    let deadline = Instant::now() + Duration::from_secs(5);
    handle.deliver("ping", b"{}", deadline)?;
    handle.fetch("/", deadline)?;

    // A stopped gateway drops the connection it accepts instead of serving it.
    wait_for_gateway(|| runtime.port())?;
    Ok(())
}

#[test]
fn reports_failures_until_the_gateway_stops() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (recorder, events) = recorded_events();
    let handle = Handle(Arc::new(shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        recorder,
    )?));

    handle.report_failure("running".to_owned());
    handle.0.stopped.cancel();
    handle.report_failure("stopped".to_owned());

    assert_eq!(
        events.try_recv()?,
        Event::Failed {
            message: "running".to_owned()
        }
    );
    assert!(events.try_recv().is_err());
    Ok(())
}

/// Run in its own process by `ends_the_process_once_its_port_is_unavailable`.
#[test]
#[ignore = "ends its process"]
#[cfg(target_os = "android")]
fn abandons_an_unavailable_port() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let shared = shared(
        directory.path(),
        Arc::new(UnreachableHandler),
        &tokio,
        Events::new(|event| eprintln!("{event:?}")),
    )?;
    let occupied = TcpListener::bind(("127.0.0.1", 0))?;
    let port = occupied.local_addr()?.port();

    let _ = replace_lost_listener(
        &shared,
        Listener {
            port,
            ..Listener::bind(0)?
        },
    );
    Ok(())
}

#[test]
#[cfg(target_os = "android")]
fn ends_the_process_once_its_port_is_unavailable() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::process::ExitStatusExt;

    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "--ignored",
            "--nocapture",
            "gateway::tests::abandons_an_unavailable_port",
        ])
        .output()?;

    let events = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.signal(), Some(libc::SIGABRT), "{events}");
    assert!(
        events.contains("gateway listener could not recover"),
        "{events}"
    );
    Ok(())
}
