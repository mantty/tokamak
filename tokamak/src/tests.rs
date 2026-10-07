use crate::certificates::Certificates;
use crate::dispatcher::{
    Dispatcher, WorkerLoader, WorkerResolver, configure_worker_loader, execute_request, load_worker,
};
use crate::fs::VirtualFileSystem;
use crate::gateway::{
    Handler, HandlerError, Job, JobResponse, Shared, WebSocketInbound, WebSocketOutbound,
    bind_replacement_listener, close_connections, execute_job, listener_was_closed,
    lock_connections, probe_gateway, serve_connection, wait_for_gateway, websocket_channels,
};
use crate::lifecycle_events::{Event, Events};
use crate::packaging::{PackageLayout, WorkerManifest, write_worker};
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::transport::{HttpBody, HttpRequest, queue_websocket_message};
use flume::{Receiver, Sender};
use hyper::header::HeaderMap;
use rquickjs::{ArrayBuffer, Context, Function, Module, Object, Runtime as JsRuntime, TypedArray};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const HOST: &str = "example.test";

const WEBSOCKET_WORKER: &[u8] = br#"
export default {
  async fetch() {
    const [client, server] = Object.values(new WebSocketPair());
    server.accept();
    server.addEventListener("message", (event) => server.send(`pong ${event.data}`));
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#;

const CACHE_WORKER: &[u8] = br#"
export default {
  async fetch(request) {
    const cache = await caches.open("pages");
    const key = "https://example.test/page";
    if (request.method === "PUT") {
      await cache.put(key, new Response("cached", { headers: { "x-cache": "hit" } }));
      return new Response("stored");
    }
    const entry = await cache.match(key);
    return new Response(entry ? `${await entry.text()} ${entry.headers.get("x-cache")}` : "miss");
  },
};
"#;

const GLOBAL_WORKER: &[u8] = br#"
const encoded = new TextEncoder().encode("ready");
const decoded = new TextDecoder().decode(encoded);
export default {
  async fetch() {
    return new Response(null, { status: decoded === "ready" ? 204 : 500 });
  },
};
"#;

const STREAM_WORKER: &[u8] = br#"
export default {
  fetch: async request => {
    if (new Uint8Array(await request.arrayBuffer()).join() !== "9,8,7") {
      throw new Error("request body was not transferred as bytes");
    }
    const chunks = [new Uint8Array([1, 2]), new Uint8Array([3, 4])];
    const body = new ReadableStream({
      pull(controller) {
        const chunk = chunks.shift();
        if (chunk) controller.enqueue(chunk);
        else controller.close();
      },
    });
    return new Response(body, { headers: { "content-type": "application/octet-stream" } });
  },
};
"#;

/// Answers `job` with `handler` on a blocking Tokio thread, as the gateway does.
fn handle_job(
    tokio: &tokio::runtime::Runtime,
    handler: Arc<dyn Handler>,
    job: Job,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio.spawn_blocking(move || {
        handler
            .handle(job, &CancellationToken::new())
            .map_err(|error| error.to_string())
    })
}

/// The body of `dispatcher`'s buffered response to `method /page`.
fn respond(
    tokio: &tokio::runtime::Runtime,
    dispatcher: &Arc<Dispatcher>,
    method: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let (response, responses) = flume::bounded(1);
    let job = Job {
        request: request(method, "/page"),
        response,
        websocket: None,
    };
    tokio.block_on(handle_job(tokio, dispatcher.clone(), job))??;
    let JobResponse::Http(response) = responses.recv()? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    let HttpBody::Buffered(body) = response.body else {
        return Err("Worker response was streamed".into());
    };
    Ok(String::from_utf8(body)?)
}

#[test]
fn keeps_cache_entries_for_the_life_of_their_runtime() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(CACHE_WORKER, directory.path())?;
    let tokio = tokio::runtime::Runtime::new()?;
    let running = Dispatcher::new(worker.clone(), runtime_config());
    let restarted = Dispatcher::new(worker, runtime_config());

    assert_eq!(respond(&tokio, &running, "PUT")?, "stored");
    assert_eq!(respond(&tokio, &running, "GET")?, "cached hit");
    assert_eq!(respond(&tokio, &restarted, "GET")?, "miss");
    Ok(())
}

fn request(method: &str, path: &str) -> HttpRequest {
    HttpRequest {
        persistent: true,
        method: method.to_owned(),
        target: path.to_owned(),
        url: format!("https://example.test{path}"),
        headers: HeaderMap::new(),
        body: None,
    }
}

#[test]
fn routes_worker_websocket_messages_through_the_native_bridge()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker_bundle = WorkerBundle::of_source(WEBSOCKET_WORKER, directory.path())?;
    let dispatcher = Dispatcher::new(worker_bundle, runtime_config());
    let (response_sender, response_receiver) = flume::bounded(1);
    let (websocket, bridge) = websocket_channels()?;
    let incoming_sender = bridge.incoming;
    let outgoing_receiver = bridge.outgoing;
    let tokio = tokio::runtime::Runtime::new()?;
    let worker = handle_job(
        &tokio,
        dispatcher,
        Job {
            request: request("GET", "/socket"),
            response: response_sender,
            websocket: Some(websocket),
        },
    );

    let response = response_receiver.recv_timeout(Duration::from_secs(1));
    if let Err(error) = response {
        let worker_error = tokio.block_on(worker)?;
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
    tokio.block_on(worker)??;
    Ok(())
}

#[test]
fn streams_worker_response_chunks_without_buffering_the_body()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker_bundle = WorkerBundle::of_source(STREAM_WORKER, directory.path())?;
    let dispatcher = Dispatcher::new(worker_bundle, runtime_config());
    let mut request = request("POST", "/upload");
    request.body = Some(vec![9, 8, 7]);
    let (response_sender, response_receiver) = flume::bounded(1);
    let tokio = tokio::runtime::Runtime::new()?;
    let worker = handle_job(
        &tokio,
        dispatcher,
        Job {
            request,
            response: response_sender,
            websocket: None,
        },
    );

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
    tokio.block_on(worker)??;
    Ok(())
}

#[test]
fn loads_split_worker_modules_through_the_quickjs_loader() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("chunks"))?;
    std::fs::write(
        directory.path().join("entry.js"),
        br#"import worker from "./chunks/worker.js"; export default worker;"#,
    )?;
    std::fs::write(
        directory.path().join("chunks/worker.js"),
        br"export default { fetch() {} };",
    )?;
    let manifest = WorkerManifest::es_modules("entry.js", &["chunks/worker.js"]);
    let app = PackageLayout::new(directory.path().join("app"));
    write_worker(&app, directory.path(), &manifest)?;
    let worker = WorkerBundle::new(manifest, app);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    executor.block_on(async {
        let runtime = rquickjs::AsyncRuntime::new()?;
        runtime
            .set_loader(
                WorkerResolver,
                WorkerLoader {
                    bundle: worker.clone(),
                    storage: None,
                },
            )
            .await;
        let context = rquickjs::AsyncContext::full(&runtime).await?;
        context
            .async_with(async |ctx| {
                let _: Object = load_worker(&ctx, &worker).await?;
                Ok::<_, Error>(())
            })
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}

const CALL_WORKER: &[u8] = br#"
export default {
  async fetch(request, env, ctx) {
    const path = new URL(request.url).pathname;
    if (path === "/tokamak/missing") return new Response(null, { status: 404 });
    if (path === "/tokamak/broken") throw new Error("handler exploded");
    if (path === "/tokamak/slow") await new Promise((resolve) => setTimeout(resolve, 5000));
    if (path === "/tokamak/relay") {
      const { status } = await fetch((await request.json()).url);
      return new Response(String(status), { status });
    }
    if (path === "/tokamak/later") {
      const { notify } = await request.json();
      ctx.waitUntil(new Promise((resolve) => setTimeout(resolve, 50)).then(() => fetch(notify)));
      return Response.json(null);
    }
    if (path === "/tokamak/late") {
      const { notify } = await request.json();
      ctx.waitUntil(new Promise((resolve) => setTimeout(resolve, 400)).then(() => fetch(notify)));
      await new Promise((resolve) => setTimeout(resolve, 300));
      return Response.json(null);
    }
    return Response.json({
      method: request.method,
      path,
      type: request.headers.get("content-type"),
      body: await request.text(),
    });
  },
};
"#;

fn call_runtime()
-> Result<(crate::gateway::Runtime, tempfile::TempDir), Box<dyn std::error::Error + Send + Sync>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(CALL_WORKER, directory.path())?;
    let dispatcher = Dispatcher::new(worker, runtime_config());
    let runtime = crate::gateway::Runtime::start(
        dispatcher,
        certificates(directory.path())?,
        HOST.to_owned(),
        Events::new(drop),
    )?;
    Ok((runtime, directory))
}

fn call_worker(
    name: &str,
    timeout: Duration,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let (runtime, _directory) = call_runtime()?;
    let body = runtime.call(name, r#"{"id":"1"}"#, timeout)?;
    Ok(serde_json::from_slice(&body)?)
}

#[test]
fn posts_a_runtime_call_to_the_worker_fetch_handler()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert_eq!(
        call_worker("push", Duration::from_secs(5))?,
        serde_json::json!({
            "method": "POST",
            "path": "/tokamak/push",
            "type": "application/json",
            "body": r#"{"id":"1"}"#,
        })
    );
    Ok(())
}

#[test]
fn fails_a_runtime_call_the_worker_does_not_answer_with_200()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Err(error) = call_worker("missing", Duration::from_secs(5)) else {
        return Err("the call succeeded".into());
    };
    assert_eq!(error.to_string(), "/tokamak/missing responded 404");
    Ok(())
}

#[test]
fn fails_a_runtime_call_the_worker_throws_in()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Err(error) = call_worker("broken", Duration::from_secs(5)) else {
        return Err("the call succeeded".into());
    };
    assert_eq!(error.to_string(), "/tokamak/broken responded 500");
    Ok(())
}

/// A URL whose first request's line arrives on the returned receiver.
fn notify_listener()
-> Result<(String, flume::Receiver<String>), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = format!("http://127.0.0.1:{}/done", listener.local_addr()?.port());
    let (arrived, arrival) = flume::bounded(1);
    thread::spawn(move || -> io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        let _ = arrived.send(line);
        stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
    });
    Ok((url, arrival))
}

/// A URL answering its requests with `statuses` in turn; each request's arrival
/// reaches the returned receiver.
fn status_server(
    statuses: &'static [u16],
) -> Result<(String, flume::Receiver<()>), Box<dyn std::error::Error + Send + Sync>> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = format!("http://127.0.0.1:{}/", listener.local_addr()?.port());
    let (arrived, arrivals) = flume::unbounded();
    thread::spawn(move || -> io::Result<()> {
        for status in statuses {
            let (mut stream, _) = listener.accept()?;
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            while reader.read_line(&mut line)? > 2 {
                line.clear();
            }
            let _ = arrived.send(());
            write!(
                stream,
                "HTTP/1.1 {status} Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )?;
        }
        Ok(())
    });
    Ok((url, arrivals))
}

/// Calls the Worker's relay endpoint against [`status_server`], returning the
/// response body or error, and the number of attempts.
fn relay_call(
    statuses: &'static [u16],
    timeout: Duration,
) -> Result<(String, usize), Box<dyn std::error::Error + Send + Sync>> {
    let (url, arrivals) = status_server(statuses)?;
    let (runtime, _directory) = call_runtime()?;
    let body = serde_json::json!({ "url": url }).to_string();
    let outcome = match runtime.call("relay", &body, timeout) {
        Ok(response) => String::from_utf8(response)?,
        Err(error) => error.to_string(),
    };
    Ok((outcome, arrivals.drain().count()))
}

#[test]
fn retries_a_failed_runtime_call() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert_eq!(
        relay_call(&[503, 200], Duration::from_secs(10))?,
        ("200".to_owned(), 2)
    );
    Ok(())
}

#[test]
fn fails_a_runtime_call_after_three_attempts()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert_eq!(
        relay_call(&[503, 503, 503, 200], Duration::from_secs(10))?,
        ("/tokamak/relay responded 503".to_owned(), 3)
    );
    Ok(())
}

#[test]
fn retries_a_runtime_call_only_while_its_timeout_allows()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    assert_eq!(
        relay_call(&[503, 200], Duration::from_millis(900))?,
        ("/tokamak/relay responded 503".to_owned(), 1)
    );
    Ok(())
}

#[test]
fn runs_wait_until_work_after_a_runtime_call_responds()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (notify, arrival) = notify_listener()?;
    let (runtime, _directory) = call_runtime()?;

    let body = runtime.call(
        "later",
        &serde_json::json!({ "notify": notify }).to_string(),
        Duration::from_secs(5),
    )?;

    assert_eq!(body, b"null");
    assert_eq!(
        arrival.recv_timeout(Duration::from_secs(5))?,
        "GET /done HTTP/1.1\r\n"
    );
    Ok(())
}

#[test]
fn runs_wait_until_work_after_a_runtime_call_times_out()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (notify, arrival) = notify_listener()?;
    let (runtime, _directory) = call_runtime()?;

    let Err(error) = runtime.call(
        "late",
        &serde_json::json!({ "notify": notify }).to_string(),
        Duration::from_millis(100),
    ) else {
        return Err("the call succeeded".into());
    };

    assert_eq!(
        error.to_string(),
        "/tokamak/late did not respond within 100ms"
    );
    assert_eq!(
        arrival.recv_timeout(Duration::from_secs(5))?,
        "GET /done HTTP/1.1\r\n"
    );
    Ok(())
}

#[test]
fn fails_a_runtime_call_that_outlasts_its_timeout()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let started = std::time::Instant::now();

    let Err(error) = call_worker("slow", Duration::from_millis(100)) else {
        return Err("the call succeeded".into());
    };

    assert_eq!(
        error.to_string(),
        "/tokamak/slow did not respond within 100ms"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
fn refuses_a_runtime_call_name_that_is_not_a_path_segment()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for name in ["", "push/../admin", "push?x=1"] {
        let Err(error) = call_worker(name, Duration::from_secs(5)) else {
            return Err(format!("the call to {name:?} succeeded").into());
        };
        assert!(
            error.to_string().contains("invalid runtime call name"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn loads_builtins_without_source_compilation() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = JsRuntime::new()?;
    configure_worker_loader(
        &runtime,
        &WorkerBundle::new(
            WorkerManifest::es_modules("entry.js", &[]),
            PackageLayout::new(directory.path()),
        ),
    );
    let context = Context::full(&runtime)?;
    context.with(|ctx| -> rquickjs::Result<()> {
        let environment = Object::new(ctx.clone())?;
        environment.set("FLAG", "request environment")?;
        ctx.globals().set("__tokamak_env", environment)?;
        let vfs = Arc::new(Mutex::new(VirtualFileSystem::new(crate::fs::Bundle::new(
            directory.path(),
        ))));
        crate::fs::install(&ctx, &vfs)?;
        crate::compat::initialize(&ctx)?;
        let module: Object = Module::import(&ctx, "cloudflare:workers")?.finish()?;
        let env: Object = module.get("env")?;
        assert_eq!(env.get::<_, String>("FLAG")?, "request environment");
        let streams: Object = Module::import(&ctx, "node:stream")?.finish()?;
        let writable: rquickjs::Constructor = streams.get("Writable")?;
        let events: Object = Module::import(&ctx, "node:events")?.finish()?;
        let emitter: Object = events.get("EventEmitter")?;
        let instance: Object = writable.construct(())?;
        assert!(instance.is_instance_of(&emitter));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn initializes_web_globals_before_worker_module_evaluation()
-> Result<(), Box<dyn std::error::Error>> {
    let JobResponse::Http(response) = run_worker(GLOBAL_WORKER, request("GET", "/"))? else {
        return Err("Worker returned a non-HTTP response".into());
    };
    assert_eq!(response.status, 204);
    Ok(())
}

#[test]
fn rejects_a_handler_result_that_is_not_a_response() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"export default { fetch: () => ({ status: 200, headers: [] }) };";
    let error = run_worker(source, request("GET", "/"))
        .err()
        .ok_or("a plain object was sent as the response")?;
    assert!(
        error
            .to_string()
            .contains("Incorrect type for Promise: the Promise did not resolve to 'Response'."),
        "{error}"
    );
    Ok(())
}

/// What the Worker compiled from `source` sends for `request`.
fn run_worker(
    source: &[u8],
    request: HttpRequest,
) -> Result<JobResponse, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(source, directory.path())?;
    let (response, responses) = flume::bounded(1);
    let job = Job {
        request,
        response,
        websocket: None,
    };
    let tokio = tokio::runtime::Runtime::new()?;
    tokio.block_on(tokio.spawn_blocking(move || {
        execute_request(&worker, &runtime_config(), job, &CancellationToken::new())
    }))??;
    Ok(responses.recv()?)
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
        &WorkerBundle::new(
            WorkerManifest::es_modules("entry.js", &[]),
            PackageLayout::new(directory.path()),
        ),
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
    let (incoming, incoming_receiver) = flume::bounded(2);
    let mut fragmented = None;

    queue_websocket_message(&incoming, &mut fragmented, false, 0x1, b"ping ".to_vec())?;
    queue_websocket_message(&incoming, &mut fragmented, true, 0x0, b"42".to_vec())?;
    queue_websocket_message(&incoming, &mut fragmented, true, 0x2, vec![1, 2, 3])?;

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
    let (started_sender, started_receiver) = flume::bounded(2);
    let (first_release_sender, first_release_receiver) = flume::bounded(1);
    let (second_release_sender, second_release_receiver) = flume::bounded(1);

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
fn reports_handler_failures_through_the_event_listener()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    struct FailingHandler;
    impl Handler for FailingHandler {
        fn handle(&self, _: Job, _: &CancellationToken) -> Result<(), HandlerError> {
            Err("handler exploded".into())
        }
    }
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let (sink, events) = flume::unbounded();
    let shared = Shared {
        handler: Arc::new(FailingHandler),
        certificates: certificates(directory.path())?,
        host: HOST.to_owned(),
        tokio: tokio.handle().clone(),
        port: AtomicU16::new(0),
        stopped: CancellationToken::new(),
        connections: Mutex::new(Vec::new()),
        events: Events::new(move |event| {
            let _ = sink.send(event);
        }),
    };
    let (response, responses) = flume::bounded(1);

    execute_job(
        &shared,
        Job {
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
    assert!(
        matches!(
            events.as_slice(),
            [Event::RequestFailed { message }]
                if message.starts_with("Worker error: ") && message.ends_with("handler exploded")
        ),
        "unexpected events: {events:?}"
    );
    Ok(())
}

#[test]
fn reports_connection_failures_through_the_event_listener()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    struct UnreachableHandler;
    impl Handler for UnreachableHandler {
        fn handle(&self, _: Job, _: &CancellationToken) -> Result<(), HandlerError> {
            Err("handler must not run".into())
        }
    }
    let directory = tempfile::tempdir()?;
    let (sink, events) = flume::unbounded();
    let runtime = crate::gateway::Runtime::start(
        Arc::new(UnreachableHandler),
        certificates(directory.path())?,
        HOST.to_owned(),
        Events::new(move |event| {
            let _ = sink.send(event);
        }),
    )?;
    assert_eq!(
        events.recv_timeout(Duration::from_secs(5))?,
        Event::Listening {
            port: runtime.port()
        }
    );

    // Hanging up before sending headers fails the connection thread's request read.
    drop(TcpStream::connect(("127.0.0.1", runtime.port()))?);

    let event = events.recv_timeout(Duration::from_secs(5))?;
    assert!(
        matches!(&event, Event::RequestFailed { message } if !message.is_empty()),
        "unexpected event: {event:?}"
    );
    Ok(())
}

fn certificates(directory: &Path) -> io::Result<Arc<Certificates>> {
    Certificates::start(directory.join("state"), HOST.to_owned())
        .map(Arc::new)
        .map_err(io::Error::other)
}

#[test]
fn shutdown_closes_registered_connections() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
{
    let directory = tempfile::tempdir()?;
    let tokio = tokio::runtime::Builder::new_current_thread().build()?;
    let shared = Arc::new(Shared {
        handler: Dispatcher::new(
            WorkerBundle::new(
                WorkerManifest::es_modules("entry.js", &[]),
                PackageLayout::new(PathBuf::default()),
            ),
            runtime_config(),
        ),
        certificates: certificates(directory.path())?,
        host: HOST.to_owned(),
        tokio: tokio.handle().clone(),
        port: AtomicU16::new(0),
        stopped: CancellationToken::new(),
        connections: Mutex::new(Vec::new()),
        events: Events::new(|_| {}),
    });
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    let connection_shared = Arc::clone(&shared);
    let connection = thread::spawn(move || serve_connection(&connection_shared, server));

    while lock_connections(&shared).is_empty() {
        thread::yield_now();
    }
    shared.stopped.cancel();
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

fn spawn_blocking_request(
    tokio: &tokio::runtime::Runtime,
    started: Sender<()>,
    release: Receiver<()>,
) -> tokio::task::JoinHandle<Result<(), Box<dyn std::error::Error + Send + Sync>>> {
    tokio.spawn_blocking(move || {
        started.send(())?;
        release.recv_timeout(Duration::from_secs(1))?;
        Ok(())
    })
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        assets: None,
        cache: Arc::default(),
        environment: BTreeMap::new(),
        storage: None,
    }
}
