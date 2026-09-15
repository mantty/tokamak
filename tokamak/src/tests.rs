use crate::dispatcher::{
    AssetManifest, AssetService, Dispatcher, WorkerLoader, WorkerResolver,
    configure_worker_loader, execute_request, load_worker,
};
use crate::fs::VirtualFileSystem;
use crate::gateway::{
    GatewayCertificates, GatewayConfig, Job, JobResponse, Lifecycle, Shared, WebSocketBridge,
    WebSocketInbound, WebSocketJob, WebSocketOutbound, bind_replacement_listener,
    close_connections, listener_was_closed, lock_connections, probe_gateway, serve_connection,
    wait_for_gateway,
};
use crate::quickjs::{Assets, Error, RuntimeConfig, WorkerBundle};
use crate::transport::{HttpBody, HttpRequest, HttpResponse, queue_websocket_message};
use rquickjs::{ArrayBuffer, Context, Function, Module, Object, Runtime as JsRuntime, TypedArray};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const WEBSOCKET_WORKER: &[u8] = br#"
globalThis.Request = class { constructor(url, init = {}) { this.url = url; this.method = init.method ?? "GET"; this.headers = init.headers ?? {}; this.body = init.body; } };
globalThis.Response = class { constructor(body = null, init = {}) { this.status = init.status ?? 200; this.headers = new Map(); this.webSocket = init.webSocket; } async text() { return ""; } };
class Socket {
  constructor() {
    this.__tokamak_outbox = [];
    this.__tokamak_peer = undefined;
    this.__tokamak_listener = undefined;
    this.__tokamak_receive = (data) => this.__tokamak_listener?.({ data });
    this.__tokamak_close = () => {};
  }
  accept() {}
  addEventListener(name, listener) { if (name === "message") this.__tokamak_listener = listener; }
  send(data) { this.__tokamak_peer.__tokamak_outbox.push({ type: "message", binary: false, data }); }
}
globalThis.WebSocketPair = class {
  constructor() {
    this[0] = new Socket();
    this[1] = new Socket();
    this[0].__tokamak_peer = this[1];
    this[1].__tokamak_peer = this[0];
  }
};
export default {
  async fetch() {
    const pair = new WebSocketPair();
    const client = pair[0];
    const server = pair[1];
    server.accept();
    server.addEventListener("message", (event) => server.send(`pong ${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#;

const SLOW_WORKER: &[u8] = br#"
globalThis.Request = class { constructor(url, init = {}) { this.url = url; this.method = init.method ?? "GET"; this.headers = init.headers ?? {}; this.body = init.body; } };
globalThis.Response = class { constructor(_body = null, init = {}) { this.status = init.status ?? 200; this.headers = new Map(); } async text() { return "ok"; } };
export default {
  async fetch() {
    const deadline = Date.now() + 250;
    while (Date.now() < deadline) {}
    return new Response(null);
  }
};
"#;

const GLOBAL_WORKER: &[u8] = br#"
const encoded = new TextEncoder().encode("ready");
const decoded = new TextDecoder().decode(encoded);
export default {
  async fetch() {
    return new Response(decoded, { status: decoded === "ready" ? 204 : 500 });
  },
};
"#;

const STREAM_WORKER: &[u8] = br#"
globalThis.Request = class Request {
  constructor(url, init = {}) {
    this.url = url;
    this.method = init.method ?? "GET";
    this.headers = init.headers ?? {};
    this.body = init.body;
  }
};
globalThis.Response = class Response {
  constructor(body = null, init = {}) {
    this.status = init.status ?? 200;
    this.headers = new Map([["content-type", "application/octet-stream"]]);
    this.__stream = body;
  }
};
const stream = {
  getReader() {
    const values = [new Uint8Array([1, 2]), new Uint8Array([3, 4])];
    return {
      read() {
        const value = values.shift();
        return Promise.resolve(value ? { done: false, value } : { done: true, value: undefined });
      },
      cancel() { return Promise.resolve(); },
    };
  },
};
export default {
  fetch: async request => {
    if (!(request.body instanceof Uint8Array) || request.body[0] !== 9 || request.body[2] !== 7) {
      throw new Error("request body was not transferred as bytes");
    }
    return new Response(stream);
  },
};
"#;

#[test]
fn uses_the_resolved_asset_for_content_type() {
    let manifest = AssetManifest {
        files: BTreeMap::from([("about/index.html".to_owned(), "text/html".to_owned())]),
        html_handling: "auto-trailing-slash".to_owned(),
    };

    assert_eq!(
        manifest.path_for("/about").as_deref(),
        Some("about/index.html")
    );
    assert_eq!(manifest.content_type("about/index.html"), "text/html");
}

#[test]
fn serves_the_resolved_asset_with_its_content_type() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    std::fs::create_dir_all(root.join("about"))?;
    std::fs::write(
        root.join("asset-manifest.json"),
        br#"{"files":{"about/index.html":"text/html"},"htmlHandling":"auto-trailing-slash"}"#,
    )?;
    std::fs::write(root.join("about/index.html"), b"about")?;

    let config = RuntimeConfig {
        assets: Some(Assets {
            manifest: root.join("asset-manifest.json"),
            root: root.to_owned(),
        }),
        cache: root.join("cache"),
        environment: BTreeMap::new(),
    };
    let request = HttpRequest {
        method: "GET".to_owned(),
        target: "/about".to_owned(),
        url: "https://example.test/about".to_owned(),
        headers: BTreeMap::new(),
        body: None,
    };
    let assets = AssetService::new(config.assets.as_ref().ok_or("assets were not configured")?)?;
    let response = assets.response(&request)?.ok_or("asset was not found")?;

    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/html")
    );
    assert!(matches!(response.body, HttpBody::Buffered(body) if body == b"about"));
    Ok(())
}

#[test]
fn routes_worker_websocket_messages_through_the_native_bridge()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = crate::compile_worker(WEBSOCKET_WORKER)?;
    let worker_bundle = WorkerBundle::from_bytecode(bundle, directory.path());
    let config = websocket_config(directory.path());
    let request = websocket_request();
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    let (incoming_sender, incoming_receiver) = mpsc::sync_channel(1);
    let (outgoing_sender, outgoing_receiver) = mpsc::sync_channel(1);
    let accepting = Arc::new(AtomicBool::new(true));
    let thread = std::thread::spawn(move || {
        let lifecycle = Lifecycle::new();
        let Some(execution) = lifecycle.enter(&accepting) else {
            return Err(Error::Startup("execution was not admitted".to_owned()));
        };
        execute_request(
            &worker_bundle,
            &config,
            None,
            Job {
                request,
                response: response_sender,
                websocket: Some(WebSocketJob {
                    incoming: incoming_receiver,
                    outgoing: outgoing_sender,
                }),
            },
            &execution,
            &accepting,
        )
    });

    let response = response_receiver.recv_timeout(Duration::from_secs(1));
    if let Err(error) = response {
        let worker_error = thread.join().map_err(|_| "WebSocket worker panicked")?;
        return Err(
            format!("WebSocket worker stopped before upgrade: {error}; {worker_error:?}").into(),
        );
    }
    assert!(matches!(response?, JobResponse::WebSocket));
    assert!(matches!(
        outgoing_receiver.recv_timeout(Duration::from_secs(1))?,
        WebSocketOutbound::Ready
    ));
    incoming_sender.send(WebSocketInbound::Message {
        binary: false,
        payload: b"ping 42".to_vec(),
    })?;
    match outgoing_receiver.recv_timeout(Duration::from_secs(1))? {
        WebSocketOutbound::Message { binary, payload } => {
            assert!(!binary);
            assert_eq!(payload, b"pong ping 42");
        }
        WebSocketOutbound::Close { .. } => panic!("Worker closed the WebSocket"),
        WebSocketOutbound::Ready => panic!("Worker completed without a response"),
    }
    assert!(matches!(
        outgoing_receiver.recv_timeout(Duration::from_secs(1))?,
        WebSocketOutbound::Ready
    ));
    incoming_sender.send(WebSocketInbound::Close {
        code: 1000,
        reason: String::new(),
    })?;
    thread.join().map_err(|_| "WebSocket worker panicked")??;
    Ok(())
}

#[test]
fn streams_worker_response_chunks_without_buffering_the_body()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let bundle = crate::compile_worker(STREAM_WORKER)?;
    let worker_bundle = WorkerBundle::from_bytecode(bundle, directory.path());
    let config = websocket_config(directory.path());
    let mut request = websocket_request();
    request.body = Some(vec![9, 8, 7]);
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    let accepting = Arc::new(AtomicBool::new(true));
    let thread = std::thread::spawn(move || {
        let lifecycle = Lifecycle::new();
        let Some(execution) = lifecycle.enter(&accepting) else {
            return Err(Error::Startup("execution was not admitted".to_owned()));
        };
        execute_request(
            &worker_bundle,
            &config,
            None,
            Job {
                request,
                response: response_sender,
                websocket: None,
            },
            &execution,
            &accepting,
        )
    });

    let response = response_receiver.recv_timeout(Duration::from_secs(1))?;
    let JobResponse::Http(response) = response else {
        return Err("Worker returned a non-HTTP response".into());
    };
    let HttpBody::Stream(body) = response.body else {
        return Err("Worker response was not streamed".into());
    };
    assert_eq!(body.recv()?.map_err(io::Error::other)?, vec![1, 2]);
    assert_eq!(body.recv()?.map_err(io::Error::other)?, vec![3, 4]);
    assert!(body.recv().is_err());
    thread.join().map_err(|_| "stream worker panicked")??;
    Ok(())
}

#[test]
fn loads_split_worker_modules_through_the_quickjs_loader() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let entry = crate::compile_module(
        "entry.js",
        br#"import worker from "./chunks/worker.js"; export default worker;"#,
    )?;
    let chunk = crate::compile_module("chunks/worker.js", br"export default { fetch() {} };")?;
    std::fs::create_dir_all(directory.path().join("chunks"))?;
    std::fs::write(directory.path().join("entry.js.qjs"), entry)?;
    std::fs::write(directory.path().join("chunks/worker.js.qjs"), chunk)?;
    let worker = WorkerBundle::from_modules("entry.js", directory.path(), directory.path());
    let runtime = JsRuntime::new()?;
    runtime.set_loader(
        WorkerResolver,
        WorkerLoader {
            bundle: worker.clone(),
        },
    );
    let context = Context::full(&runtime)?;

    context.with(|ctx| {
        let _: Function = load_worker(&ctx, &worker)?;
        Ok::<_, Error>(())
    })?;
    Ok(())
}

#[test]
fn loads_builtins_without_source_compilation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = JsRuntime::new()?;
    configure_worker_loader(
        &runtime,
        &WorkerBundle::from_modules("entry.js", directory.path(), directory.path()),
    );
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> rquickjs::Result<()> {
        let environment = Object::new(ctx.clone())?;
        environment.set("FLAG", "request environment")?;
        ctx.globals().set("__tokamak_env", environment)?;
        let module: Object = Module::import(&ctx, "cloudflare:workers")?.finish()?;
        let env: Object = module.get("env")?;
        assert_eq!(env.get::<_, String>("FLAG")?, "request environment");
        let streams: Object = Module::import(&ctx, "node:stream")?.finish()?;
        let writable: Object = streams.get("Writable")?;
        let events: Object = Module::import(&ctx, "node:events")?.finish()?;
        let emitter: Object = events.get("EventEmitter")?;
        assert_eq!(writable.get_prototype(), Some(emitter));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn initializes_web_globals_before_worker_module_evaluation()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker =
        WorkerBundle::from_bytecode(crate::compile_worker(GLOBAL_WORKER)?, directory.path());
    let accepting = Arc::new(AtomicBool::new(true));
    let lifecycle = Lifecycle::new();
    let execution = lifecycle
        .enter(&accepting)
        .ok_or("request was not admitted")?;
    let (response_sender, response_receiver) = mpsc::sync_channel(1);

    execute_request(
        &worker,
        &websocket_config(directory.path()),
        None,
        Job {
            request: websocket_request(),
            response: response_sender,
            websocket: None,
        },
        &execution,
        &accepting,
    )?;

    let JobResponse::Http(response) = response_receiver.recv()? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    assert_eq!(response.status, 204);
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn exposes_bundle_tmp_and_device_operations() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("config"))?;
    std::fs::write(
        directory.path().join("config/app.json"),
        br#"{"enabled":true}"#,
    )?;
    let runtime = JsRuntime::new()?;
    configure_worker_loader(
        &runtime,
        &WorkerBundle::from_modules("entry.js", directory.path(), directory.path()),
    );
    let context = Context::full(&runtime)?;
    context.with(|ctx| {
        let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(crate::fs::Bundle::new(
            directory.path(),
        ))));
        crate::fs::install(&ctx, &vfs)
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?;
        let object: Object = Module::import(&ctx, "node:fs")
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?
            .finish()
            .map_err(|error| Error::Engine(format!("{error}; {:?}", ctx.catch())))?;
        let read: Function = object
            .get("readFileSync")
            .map_err(|error| Error::Engine(format!("readFileSync: {error}")))?;
        let value: TypedArray<u8> = read
            .call(("/bundle/config/app.json",))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(
            value
                .as_bytes()
                .ok_or_else(|| Error::Engine("detached bundle bytes".to_owned()))?,
            br#"{"enabled":true}"#
        );
        let write: Function = object
            .get("writeFileSync")
            .map_err(|error| Error::Engine(format!("writeFileSync: {error}")))?;
        let data = ArrayBuffer::new_copy(ctx.clone(), b"value")
            .map_err(|error| Error::Engine(error.to_string()))?;
        write
            .call::<_, ()>(("/tmp/value.txt", data))
            .map_err(|error| Error::Engine(error.to_string()))?;
        let info: Function = object
            .get("statSync")
            .map_err(|error| Error::Engine(format!("statSync: {error}")))?;
        let value: Object = info
            .call(("/tmp/value.txt", true))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(
            value
                .get::<_, u64>("size")
                .map_err(|error| Error::Engine(error.to_string()))?,
            5
        );

        let open: Function = object
            .get("openSync")
            .map_err(|error| Error::Engine(format!("openSync: {error}")))?;
        let descriptor: u32 = open
            .call(("/dev/zero", "r"))
            .map_err(|error| Error::Engine(error.to_string()))?;
        let read: Function = object
            .get("readSync")
            .map_err(|error| Error::Engine(format!("readSync: {error}")))?;
        let zeros = TypedArray::<u8>::new_copy(ctx.clone(), [0_u8; 3])
            .map_err(|error| Error::Engine(error.to_string()))?;
        let read_count: u32 = read
            .call((descriptor, zeros.clone(), 0_u32, 3_u32, 0_i64))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert_eq!(read_count, 3);
        assert_eq!(zeros.as_bytes(), Some(&[0, 0, 0][..]));
        let device_info: Object = info
            .call(("/dev/zero", true))
            .map_err(|error| Error::Engine(error.to_string()))?;
        assert!(
            device_info
                .get::<_, bool>("device")
                .map_err(|error| Error::Engine(error.to_string()))?
        );
        let close: Function = object
            .get("closeSync")
            .map_err(|error| Error::Engine(format!("closeSync: {error}")))?;
        close
            .call::<_, ()>((descriptor,))
            .map_err(|error| Error::Engine(error.to_string()))?;
        Ok::<_, Error>(())
    })?;
    Ok(())
}

#[test]
fn assembles_fragmented_text_and_binary_websocket_messages()
-> Result<(), Box<dyn std::error::Error>> {
    let (incoming_sender, incoming_receiver) = mpsc::sync_channel(2);
    let (_outgoing_sender, outgoing_receiver) = mpsc::sync_channel(1);
    let bridge = WebSocketBridge {
        incoming: incoming_sender,
        outgoing: outgoing_receiver,
    };
    let mut fragmented = None;

    queue_websocket_message(&bridge, &mut fragmented, false, 0x1, b"ping ".to_vec())?;
    queue_websocket_message(&bridge, &mut fragmented, true, 0x0, b"42".to_vec())?;
    queue_websocket_message(&bridge, &mut fragmented, true, 0x2, vec![1, 2, 3])?;

    assert!(fragmented.is_none());
    assert!(matches!(
        incoming_receiver.recv()?,
        WebSocketInbound::Message { binary: false, payload } if payload == b"ping 42"
    ));
    assert!(matches!(
        incoming_receiver.recv()?,
        WebSocketInbound::Message { binary: true, payload } if payload == vec![1, 2, 3]
    ));
    Ok(())
}

#[test]
fn request_tasks_run_concurrently_on_tokio_blocking_workers()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()?;
    let (started_sender, started_receiver) = mpsc::sync_channel(2);
    let (first_release_sender, first_release_receiver) = mpsc::sync_channel(1);
    let (second_release_sender, second_release_receiver) = mpsc::sync_channel(1);

    let first = spawn_blocking_request(&tokio, started_sender.clone(), first_release_receiver);
    let second = spawn_blocking_request(&tokio, started_sender, second_release_receiver);

    let first_started = started_receiver.recv_timeout(Duration::from_secs(1));
    let second_started = started_receiver.recv_timeout(Duration::from_secs(1));
    first_release_sender.send(())?;
    second_release_sender.send(())?;
    tokio.block_on(async {
        first.await??;
        second.await??;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })?;

    first_started?;
    second_started?;
    Ok(())
}

#[test]
fn shutdown_closes_registered_connections() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let quickjs_config = RuntimeConfig {
        assets: None,
        cache: PathBuf::default(),
        environment: BTreeMap::new(),
    };
    let config = GatewayConfig {
        certificates: GatewayCertificates {
            ca: PathBuf::default(),
            certificate: PathBuf::default(),
            private_key: PathBuf::default(),
        },
        host: "example.test".to_owned(),
        require_client_certificate: false,
        port: 0,
    };
    let shared = Arc::new(Shared {
        handler: Dispatcher::new(
            WorkerBundle::from_bytecode(Vec::new(), PathBuf::default()),
            quickjs_config,
        )?,
        config,
        tokio: tokio.handle().clone(),
        port: AtomicU16::new(0),
        accepting: Arc::new(AtomicBool::new(true)),
        lifecycle: Arc::new(Lifecycle::new()),
        connections: Mutex::new(Vec::new()),
    });
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    let connection_shared = Arc::clone(&shared);
    let connection = thread::spawn(move || serve_connection(&connection_shared, server));

    while lock_connections(&shared).is_empty() {
        thread::yield_now();
    }
    shared.accepting.store(false, Ordering::Release);
    close_connections(&shared);
    let _ = connection
        .join()
        .map_err(|_| "connection thread panicked")?;
    assert!(lock_connections(&shared).is_empty());
    drop(client);
    Ok(())
}

#[test]
#[cfg(unix)]
fn closed_listener_reclaims_its_port() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let error = io::Error::from_raw_os_error(libc::EBADF);
    assert!(listener_was_closed(&error));
    drop(listener);

    let (_listener, replacement_port) = bind_replacement_listener(port)?;

    assert_eq!(replacement_port, port);
    Ok(())
}

#[test]
#[cfg(unix)]
fn closed_listener_uses_a_random_port_when_its_port_was_taken()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let _occupied = TcpListener::bind(("127.0.0.1", port))?;

    let (_listener, replacement_port) = bind_replacement_listener(port)?;

    assert_ne!(replacement_port, port);
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
fn suspension_drains_active_work_and_blocks_new_work_until_resume()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let lifecycle = Arc::new(Lifecycle::new());
    let accepting = Arc::new(AtomicBool::new(true));
    let execution = lifecycle
        .enter(&accepting)
        .ok_or("initial execution was not admitted")?;
    lifecycle.suspend();

    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (admitted_sender, admitted_receiver) = mpsc::sync_channel(1);
    let waiting_lifecycle = Arc::clone(&lifecycle);
    let waiting_accepting = Arc::clone(&accepting);
    let waiting = thread::spawn(move || {
        started_sender
            .send(())
            .map_err(|error| Error::Startup(error.to_string()))?;
        let Some(_execution) = waiting_lifecycle.enter(&waiting_accepting) else {
            return Err("execution was not admitted after resume".into());
        };
        admitted_sender.send(())?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    started_receiver.recv_timeout(Duration::from_secs(1))?;
    assert!(
        admitted_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    drop(execution);
    assert!(
        admitted_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );

    lifecycle.resume();
    admitted_receiver.recv_timeout(Duration::from_secs(1))?;
    waiting.join().map_err(|_| "lifecycle waiter panicked")??;
    Ok(())
}

#[test]
fn suspension_without_active_work_blocks_until_resume()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let lifecycle = Arc::new(Lifecycle::new());
    let accepting = Arc::new(AtomicBool::new(true));
    lifecycle.suspend();
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (admitted_sender, admitted_receiver) = mpsc::sync_channel(1);
    let waiting_lifecycle = Arc::clone(&lifecycle);
    let waiting_accepting = Arc::clone(&accepting);
    let waiting = thread::spawn(move || {
        started_sender.send(())?;
        admitted_sender.send(waiting_lifecycle.enter(&waiting_accepting).is_some())?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    started_receiver.recv_timeout(Duration::from_secs(1))?;
    assert!(
        admitted_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    lifecycle.resume();
    assert!(admitted_receiver.recv_timeout(Duration::from_secs(1))?);
    waiting.join().map_err(|_| "lifecycle waiter panicked")??;
    Ok(())
}

#[test]
fn stopping_releases_blocked_admission() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let lifecycle = Arc::new(Lifecycle::new());
    let accepting = Arc::new(AtomicBool::new(true));
    lifecycle.suspend();
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let (admitted_sender, admitted_receiver) = mpsc::sync_channel(1);
    let waiting_lifecycle = Arc::clone(&lifecycle);
    let waiting_accepting = Arc::clone(&accepting);
    let waiting = thread::spawn(move || {
        started_sender.send(())?;
        admitted_sender.send(waiting_lifecycle.enter(&waiting_accepting).is_some())?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    started_receiver.recv_timeout(Duration::from_secs(1))?;
    assert!(
        admitted_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    lifecycle.stop();
    assert!(!admitted_receiver.recv_timeout(Duration::from_secs(1))?);
    waiting.join().map_err(|_| "lifecycle waiter panicked")??;
    Ok(())
}

#[test]
fn suspension_allows_an_active_javascript_turn_to_finish()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let directory = tempfile::tempdir()?;
    let bundle = crate::compile_worker(SLOW_WORKER)?;
    let worker_bundle = WorkerBundle::from_bytecode(bundle, directory.path());
    let config = websocket_config(directory.path());
    let request = websocket_request();
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    let lifecycle = Arc::new(Lifecycle::new());
    let accepting = Arc::new(AtomicBool::new(true));
    let (started_sender, started_receiver) = mpsc::sync_channel(1);
    let worker_lifecycle = Arc::clone(&lifecycle);
    let worker_accepting = Arc::clone(&accepting);
    let worker = thread::spawn(move || {
        let Some(execution) = worker_lifecycle.enter(&worker_accepting) else {
            return Err(Error::Startup("execution was not admitted".to_owned()));
        };
        started_sender
            .send(())
            .map_err(|error| Error::Startup(error.to_string()))?;
        execute_request(
            &worker_bundle,
            &config,
            None,
            Job {
                request,
                response: response_sender,
                websocket: None,
            },
            &execution,
            &worker_accepting,
        )
    });

    started_receiver.recv_timeout(Duration::from_secs(1))?;
    lifecycle.suspend();
    assert!(
        response_receiver
            .recv_timeout(Duration::from_millis(50))
            .is_err()
    );
    assert!(matches!(
        response_receiver.recv_timeout(Duration::from_secs(1))?,
        JobResponse::Http(HttpResponse { status: 200, .. })
    ));
    worker.join().map_err(|_| "worker panicked")??;
    Ok(())
}

fn spawn_blocking_request(
    tokio: &tokio::runtime::Runtime,
    started: SyncSender<()>,
    release: Receiver<()>,
) -> tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> {
    tokio.spawn_blocking(move || {
        started.send(())?;
        release.recv_timeout(Duration::from_secs(1))?;
        Ok(())
    })
}

fn websocket_config(root: &Path) -> RuntimeConfig {
    RuntimeConfig {
        assets: None,
        cache: root.join("cache"),
        environment: BTreeMap::new(),
    }
}

fn websocket_request() -> HttpRequest {
    HttpRequest {
        method: "GET".to_owned(),
        target: "/socket".to_owned(),
        url: "https://example.test/socket".to_owned(),
        headers: BTreeMap::new(),
        body: None,
    }
}
