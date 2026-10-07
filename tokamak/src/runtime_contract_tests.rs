use reqwest::header::{HeaderMap, HeaderValue};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::assets::Assets;
use crate::dispatcher::Dispatcher;
use crate::env_vars::StorageBinding;
use crate::gateway::{Handler, Job, JobResponse};
use crate::packaging::{
    AssetManifest, HtmlHandling, ModuleType, NotFoundHandling, PackageLayout, WorkerManifest,
    write_worker,
};
use crate::quickjs::{RuntimeConfig, WorkerBundle};
use crate::storage::Storage;
use crate::transport::{HttpBody, HttpRequest};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/quickjs_runtime")
}

/// The Worker starting from `entry`, packaged under `output` with the other
/// module files in its directory.
fn entry_worker(entry: &Path, output: &Path) -> TestResult<WorkerBundle> {
    let root = entry.parent().ok_or("entry has no directory")?;
    let name = entry.file_name().and_then(|name| name.to_str());
    module_worker(root, name.ok_or("entry has no name")?, output)
}

/// The Worker of the module files in `root`, packaged under `output`: `.mjs`
/// and `.js` files are ES modules, `.txt` files Text and `.bin` files Data.
/// Other files and `node_modules` are left out.
fn module_worker(root: &Path, entry: &str, output: &Path) -> TestResult<WorkerBundle> {
    let files = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|file| file.file_name() != "node_modules");
    let mut manifest = WorkerManifest::es_modules(entry, &[]);
    for file in files {
        let file = file?;
        let module_type = match file.path().extension().and_then(|ext| ext.to_str()) {
            Some("mjs" | "js") => ModuleType::EsModule,
            Some("txt") => ModuleType::Text,
            Some("bin") => ModuleType::Data,
            _ => continue,
        };
        let name = file.path().strip_prefix(root)?.to_string_lossy();
        manifest
            .modules
            .insert(name.replace('\\', "/"), module_type);
    }
    let layout = PackageLayout::new(output);
    write_worker(&layout, root, &manifest)?;
    Ok(WorkerBundle::new(manifest, layout))
}

fn request(worker: &WorkerBundle, flag: &str) -> TestResult<Vec<u8>> {
    let environment = BTreeMap::from([("FLAG".to_owned(), serde_json::json!(flag))]);
    request_with(worker, runtime_config(environment, None))
}

/// The body of `worker`'s 200 response, with `config`, to `GET /`.
fn request_with(worker: &WorkerBundle, config: RuntimeConfig) -> TestResult<Vec<u8>> {
    let (status, _, body, _) = fixture_request(worker, config, http_request("GET", "/", None))?;
    assert_eq!(status, 200);
    Ok(body)
}

/// A configuration of `environment` and `assets`, with a cache of its own.
fn runtime_config(
    environment: BTreeMap<String, serde_json::Value>,
    assets: Option<Arc<Assets>>,
) -> RuntimeConfig {
    RuntimeConfig {
        assets,
        cache: Arc::default(),
        environment,
        storage: None,
    }
}

#[test]
fn node_web_stream_adapters_close_and_flush_vectors() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import { Writable, Duplex } from "node:stream";
export default { async fetch() {
  const chunks = [];
  const writer = Writable.toWeb(new Writable({
    write(chunk, encoding, callback) { chunks.push(...chunk); callback(); },
  })).getWriter();
  await writer.write(new Uint8Array([1, 2]));
  await writer.close();
  for (const duplex of [false, true]) {
    const web = new WritableStream({ write(chunk) { chunks.push(...chunk); } });
    const node = duplex ? Duplex.fromWeb({
      readable: new ReadableStream({ start(c) { c.close(); } }), writable: web,
    }) : Writable.fromWeb(web);
    const complete = new Promise((resolve, reject) => { node.on("error", reject); node.on("finish", resolve); });
    node.cork();
    node.write(new Uint8Array([3]));
    node.write(new Uint8Array([4]));
    node.end();
    await complete;
    node.destroy();
  }
  const errors = [];
  const callbacks = [];
  for (const duplex of [false, true]) {
    const web = new WritableStream({ write() { throw new Error("sink rejected"); } });
    const node = duplex ? Duplex.fromWeb({
      readable: new ReadableStream({ start(c) { c.close(); } }), writable: web,
    }) : Writable.fromWeb(web);
    const complete = new Promise(resolve => { node.on("error", error => errors.push(error.message)); node.on("close", resolve); });
    node.cork();
    node.write(new Uint8Array([3]), error => callbacks.push(error?.message));
    node.write(new Uint8Array([4]), error => callbacks.push(error?.message));
    node.end();
    await complete;
  }
  return Response.json({ chunks, errors, callbacks });
} };
"#,
        directory.path(),
    )?;
    assert_eq!(request(&worker, "enabled")?, br#"{"chunks":[1,2,3,4,3,4],"errors":["sink rejected","sink rejected"],"callbacks":["sink rejected","sink rejected","sink rejected","sink rejected"]}"#);
    Ok(())
}

#[test]
fn node_web_duplex_failure_closes_the_live_peer() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import { Duplex } from "node:stream";
export default { async fetch() {
  const cleanup = [];
  for (const failing of ["readable", "writable"]) {
    let input, output;
    const node = Duplex.fromWeb({
      readable: new ReadableStream({
        start(controller) { input = controller; },
        cancel(error) { cleanup.push(["readable", error.message]); },
      }),
      writable: new WritableStream({
        start(controller) { output = controller; },
        abort(error) { cleanup.push(["writable", error.message]); },
      }),
    });
    const closed = new Promise(resolve => { node.on("error", () => {}); node.on("close", resolve); });
    (failing === "readable" ? input : output).error(new Error(failing));
    await closed;
  }
  return Response.json(cleanup);
} };
"#,
        directory.path(),
    )?;
    assert_eq!(
        request(&worker, "enabled")?,
        br#"[["writable","readable"],["readable","writable"]]"#
    );
    Ok(())
}

#[test]
fn brotli_small_output_buffers_drain_without_recursion() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import { brotliCompressSync, createBrotliDecompress } from "node:zlib";
import { Buffer } from "node:buffer";
export default { async fetch() {
  const plain = "incremental data ".repeat(8192);
  const encoded = brotliCompressSync(plain);
  const decoder = createBrotliDecompress({ chunkSize: 64 });
  const chunks = [];
  const done = new Promise((resolve, reject) => {
    decoder.on("data", chunk => chunks.push(chunk));
    decoder.once("end", resolve);
    decoder.once("error", reject);
  });
  for (const byte of encoded) decoder.write(Buffer.from([byte]));
  decoder.end();
  await done;
  return Response.json({ restored: Buffer.concat(chunks).toString() === plain });
} };
"#,
        directory.path(),
    )?;
    assert_eq!(request(&worker, "enabled")?, br#"{"restored":true}"#);
    Ok(())
}

#[test]
fn due_timers_preserve_order_and_microtask_checkpoints() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
export default { async fetch() {
  for (let round = 0; round < 20; round++) {
    const order = [];
    const done = [];
    const cancelled = setTimeout(() => { throw new Error("cancelled timer ran"); }, 0);
    clearTimeout(cancelled);
    for (let i = 0; i < 50; i++) done.push(new Promise(resolve => setTimeout(() => {
      order.push(i * 2);
      queueMicrotask(() => order.push(i * 2 + 1));
      resolve();
    }, 0)));
    await Promise.all(done);
    if (order.length !== 100 || order.some((value, index) => value !== index)) throw new Error(JSON.stringify(order));
  }
  return new Response("ordered");
} };
"#,
        directory.path(),
    )?;
    assert_eq!(request(&worker, "enabled")?, b"ordered");
    Ok(())
}

#[test]
fn timers_progress_while_a_socket_waits_for_data() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut ping = [0; 4];
        stream.read_exact(&mut ping)?;
        thread::sleep(Duration::from_millis(100));
        stream.write_all(b"pong")
    });
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        format!(
            r#"
import {{ connect }} from "node:net";
export default {{ async fetch() {{
  const result = await new Promise((resolve, reject) => {{
    let timerRan = false;
    const socket = connect({{ host: "127.0.0.1", port: {port} }}, () => {{
      setTimeout(() => {{ timerRan = true; }}, 1);
      socket.write("ping");
    }});
    socket.on("data", () => {{ socket.destroy(); resolve(timerRan); }});
    socket.on("error", reject);
  }});
  return Response.json(result);
}} }};
"#
        ),
        directory.path(),
    )?;
    let actual = request(&worker, "first");
    server.join().map_err(|_| "socket server panicked")??;
    assert_eq!(actual?, b"true");
    Ok(())
}

#[test]
fn streaming_fetch_matches_cloudflare() -> TestResult {
    use std::io::BufRead;
    use std::process::Stdio;
    let mut reference = Command::new("node")
        .arg(fixture_root().join("http-reference.mjs"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let output = reference.stdout.take().ok_or("reference stdout missing")?;
    let mut line = String::new();
    io::BufReader::new(output).read_line(&mut line)?;
    let result = (|| -> TestResult {
        let expected: serde_json::Value = serde_json::from_str(&line)?;
        let port = expected["port"].as_str().ok_or("reference port missing")?;
        let directory = tempfile::tempdir()?;
        let worker = entry_worker(
            &fixture_root().join("http.mjs"),
            &directory.path().join("modules"),
        )?;
        let actual: serde_json::Value = serde_json::from_slice(&request(&worker, port)?)?;
        report_contract_difference("http", Some(&actual), &expected["expected"]);
        if actual != expected["expected"] {
            return Err("streaming HTTP contract differs".into());
        }
        Ok(())
    })();
    drop(reference.stdin.take());
    let status = reference.wait()?;
    result?;
    assert!(status.success());
    Ok(())
}

#[test]
fn timers_progress_while_fetch_waits_for_the_network() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || -> io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte)?;
            headers.push(byte[0]);
        }
        thread::sleep(Duration::from_millis(150));
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
    });
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        format!(
            r#"
export default {{ async fetch() {{
  let timerRan = false;
  const timer = setTimeout(() => {{ timerRan = true; }}, 1);
  const response = await fetch("http://{address}");
  await response.text();
  clearTimeout(timer);
  return Response.json({{ timerRan }});
}} }};
"#
        ),
        directory.path(),
    )?;
    let actual = request(&worker, "first");
    server.join().map_err(|_| "upstream panicked")??;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&actual?)?,
        serde_json::json!({ "timerRan": true })
    );
    Ok(())
}

#[test]
fn packaged_globals_precede_application_modules() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        "import { Readable } from 'node:stream'; const decoder = new TextDecoder(); const value = decoder.decode(new TextEncoder().encode('ready')); const signal = AbortSignal.any([]); export default { async fetch() { const chunks = []; for await (const chunk of Readable.from([value], { signal })) chunks.push(chunk); return new Response(chunks.join('')); } };",
        directory.path(),
    )?;
    assert_eq!(request(&worker, "first")?, b"ready");
    Ok(())
}

#[test]
fn node_http_handler_uses_tokamak_request_response_boundary() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import http from "node:http";
import { httpServerHandler } from "cloudflare:node";

const server = http.createServer((request, response) => {
  const chunks = [];
  request.on("data", chunk => chunks.push(new TextDecoder().decode(chunk)));
  request.on("end", () => {
    response.writeHead(201, { "x-tokamak-boundary": "yes" });
    response.end(`${request.method} ${request.url} ${chunks.join("")}`);
  });
});

export default httpServerHandler(server);
"#,
        directory.path(),
    )?;
    let config = RuntimeConfig {
        assets: None,
        cache: Arc::default(),
        environment: BTreeMap::new(),
        storage: None,
    };
    let request = HttpRequest {
        persistent: true,
        method: "POST".to_owned(),
        target: "/bridge?value=1".to_owned(),
        url: "https://app.tokamak.local/bridge?value=1".to_owned(),
        headers: HeaderMap::new(),
        body: Some(b"payload".to_vec()),
    };
    let (status, headers, body, _) = fixture_request(&worker, config, request)?;
    assert_eq!(status, 201);
    assert_eq!(
        headers.get("x-tokamak-boundary"),
        Some(&HeaderValue::from_static("yes"))
    );
    assert_eq!(body, b"POST /bridge?value=1 payload");
    Ok(())
}

#[test]
fn worker_entrypoint_receives_context_and_module_exports() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import { env, exports, WorkerEntrypoint } from "cloudflare:workers";
export const named = { value: 42 };
export default class App extends WorkerEntrypoint {
  fetch() {
    return Response.json({
      instance: this instanceof App,
      env: this.env.FLAG,
      importedEnv: env.FLAG,
      contextExports: Object.getOwnPropertyNames(this.ctx.exports).sort(),
      importedExports: Object.getOwnPropertyNames(exports).sort(),
      named: exports.named.value,
      context: Object.getOwnPropertyNames(this.ctx).sort(),
      props: this.ctx.props,
    });
  }
}
"#,
        directory.path(),
    )?;
    let response: serde_json::Value = serde_json::from_slice(&request(&worker, "enabled")?)?;
    assert_eq!(response["instance"], true);
    assert_eq!(response["env"], "enabled");
    assert_eq!(response["importedEnv"], "enabled");
    assert_eq!(
        response["contextExports"],
        serde_json::json!(["default", "named"])
    );
    assert_eq!(
        response["importedExports"],
        serde_json::json!(["default", "named"])
    );
    assert_eq!(response["named"], 42);
    assert_eq!(
        response["context"],
        serde_json::json!(["access", "cache", "exports", "props", "tracing"])
    );
    assert_eq!(response["props"], serde_json::json!({}));
    Ok(())
}

#[test]
fn worker_matches_cloudflare_node_compat() -> TestResult {
    let expected = node_reference("workerd-reference.mjs")?;
    let directory = tempfile::tempdir()?;
    let worker = entry_worker(&fixture_root().join("startup.mjs"), directory.path())?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "enabled")?)?;
    assert_contract_domains(&expected, &actual)
}

#[test]
fn text_and_data_modules_match_cloudflare() -> TestResult {
    let expected = node_reference("modules-reference.mjs")?;
    let directory = tempfile::tempdir()?;
    let worker = module_worker(
        &fixture_root().join("modules"),
        "modules.mjs",
        directory.path(),
    )?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "enabled")?)?;
    assert_eq!(actual, expected);
    Ok(())
}

/// The JSON a reference script prints after running a fixture in workerd.
fn node_reference(script: &str) -> TestResult<serde_json::Value> {
    let reference = Command::new("node")
        .arg(fixture_root().join(script))
        .output()?;
    if !reference.status.success() {
        return Err(format!(
            "{script} failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        )
        .into());
    }
    Ok(serde_json::from_slice(&reference.stdout)?)
}

/// Assert that every domain of `actual` equals `expected`, printing each
/// difference.
fn assert_contract_domains(expected: &serde_json::Value, actual: &serde_json::Value) -> TestResult {
    let mut failures = Vec::new();
    for (name, expected) in expected.as_object().ok_or("reference is not an object")? {
        if actual.get(name) == Some(expected) {
            continue;
        }
        failures.push(name.clone());
        report_contract_difference(name, actual.get(name), expected);
    }
    assert!(
        failures.is_empty(),
        "Cloudflare contract domains differ (details above): {failures:?}"
    );
    Ok(())
}

/// Prints a per-entry diff for one contract domain, keeping failures readable.
fn report_contract_difference(
    name: &str,
    actual: Option<&serde_json::Value>,
    expected: &serde_json::Value,
) {
    let Some(actual) = actual else {
        eprintln!("Cloudflare contract: {name}: missing, expected {expected}");
        return;
    };
    if actual == expected {
        return;
    }
    if let (Some(actual), Some(expected)) = (actual.as_array(), expected.as_array()) {
        for index in 0..actual.len().max(expected.len()) {
            report_contract_difference(
                &format!("{name}[{index}]"),
                actual.get(index),
                expected.get(index).unwrap_or(&serde_json::Value::Null),
            );
        }
        return;
    }
    let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) else {
        eprintln!("Cloudflare contract: {name}: actual={actual} expected={expected}");
        return;
    };
    let extra_keys = actual.keys().filter(|key| !expected.contains_key(*key));
    for key in expected.keys().chain(extra_keys) {
        let (actual, expected) = (actual.get(key), expected.get(key));
        if actual != expected {
            let null = serde_json::Value::Null;
            report_contract_difference(&format!("{name}.{key}"), actual, expected.unwrap_or(&null));
        }
    }
}

#[test]
fn storage_bindings_match_cloudflare() -> TestResult {
    let expected = node_reference("storage-reference.mjs")?;
    let directory = tempfile::tempdir()?;
    let worker = entry_worker(
        &fixture_root().join("storage.mjs"),
        &directory.path().join("worker"),
    )?;
    let bindings = [
        StorageBinding::D1 {
            name: "DB".to_owned(),
            id: "DB".to_owned(),
            migrations_table: "d1_migrations".to_owned(),
            migrations: Vec::new(),
        },
        StorageBinding::Kv {
            name: "KV".to_owned(),
            id: "KV".to_owned(),
        },
        StorageBinding::R2 {
            name: "R2".to_owned(),
            id: "R2".to_owned(),
        },
    ];
    let config = RuntimeConfig {
        assets: None,
        cache: Arc::default(),
        environment: BTreeMap::new(),
        storage: Some(Arc::new(Storage::open(
            &directory.path().join("storage"),
            &directory.path().join("scratch"),
            &PackageLayout::new(directory.path()),
            &bindings,
        )?)),
    };
    let actual: serde_json::Value = serde_json::from_slice(&request_with(&worker, config)?)?;
    assert_contract_domains(&expected, &actual)
}

#[test]
fn r2_stores_fetched_bodies_of_known_length() -> TestResult {
    let plain = TcpListener::bind("127.0.0.1:0")?;
    let encoded = TcpListener::bind("127.0.0.1:0")?;
    let (plain_address, encoded_address) = (plain.local_addr()?, encoded.local_addr()?);
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = plain.accept().map_err(|error| error.to_string())?;
        let mut request = [0; 1024];
        let _ = stream
            .read(&mut request)
            .map_err(|error| error.to_string())?;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello")
            .map_err(|error| error.to_string())?;
        serve_gzip_upstream(&encoded)
    });
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        format!(
            r#"
export default {{ async fetch(request, env) {{
  const plain = await env.FILES.put("plain", (await fetch("http://{plain_address}")).body);
  const encoded = await env.FILES.put("encoded", (await fetch("http://{encoded_address}")).body).catch(error => error.message);
  return Response.json({{ size: plain.size, text: await (await env.FILES.get("plain")).text(), encoded }});
}} }};
"#
        ),
        directory.path(),
    )?;
    let bindings = [StorageBinding::R2 {
        name: "FILES".to_owned(),
        id: "files".to_owned(),
    }];
    let config = RuntimeConfig {
        assets: None,
        cache: Arc::default(),
        environment: BTreeMap::new(),
        storage: Some(Arc::new(Storage::open(
            &directory.path().join("storage"),
            &directory.path().join("scratch"),
            &PackageLayout::new(directory.path()),
            &bindings,
        )?)),
    };

    let actual = request_with(&worker, config);
    server.join().map_err(|_| "upstream panicked")??;

    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&actual?)?,
        serde_json::json!({
            "size": 5,
            "text": "hello",
            "encoded": "Provided readable stream must have a known length (request/response body or readable half of FixedLengthStream)",
        })
    );
    Ok(())
}

#[test]
fn every_public_module_spelling_imports() -> TestResult {
    let names = serde_json::to_string(&crate::runtime_modules::runtime_module_names())?;
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        format!(
            r"const names = {names};
export default {{ async fetch() {{
  const empty = [];
  for (const name of names) {{
    const namespace = await import(name);
    if (Object.keys(namespace).length === 0 && namespace.default === undefined) empty.push(name);
  }}
  return new Response(JSON.stringify(empty));
}} }};
"
        ),
        directory.path(),
    )?;
    assert_eq!(request(&worker, "first")?, b"[]");
    Ok(())
}

#[test]
fn request_boundary_matches_cloudflare() -> TestResult {
    let expected = node_reference("boundary-reference.mjs")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let upstream = thread::spawn(move || serve_gzip_upstream(&listener));
    let directory = tempfile::tempdir()?;
    let worker = entry_worker(
        &fixture_root().join("boundary.mjs"),
        &directory.path().join("modules"),
    )?;
    let echo = boundary_request(&worker, port, "POST", "/echo", Some(vec![b'x'; 70000]))?;
    let stream = boundary_request(&worker, port, "GET", "/stream", None)?;
    let egress = boundary_request(&worker, port, "GET", "/egress", None)?;
    let actual = serde_json::json!({
        "echo": serde_json::from_slice::<serde_json::Value>(&echo.2)?,
        "stream": {
            "body": String::from_utf8(stream.2)?,
            "hasContentLength": stream.1.contains_key("content-length"),
        },
        "egress": serde_json::from_slice::<serde_json::Value>(&egress.2)?,
    });
    for route in ["echo", "stream", "egress"] {
        assert_eq!(
            actual.get(route),
            expected.get(route),
            "boundary contract: {route}"
        );
    }
    upstream.join().map_err(|_| "upstream fixture panicked")??;
    Ok(())
}

fn boundary_request(
    worker: &WorkerBundle,
    upstream_port: u16,
    method: &str,
    target: &str,
    body: Option<Vec<u8>>,
) -> TestResult<(u16, HeaderMap, Vec<u8>, String)> {
    let environment = BTreeMap::from([(
        "UPSTREAM_PORT".to_owned(),
        serde_json::json!(upstream_port.to_string()),
    )]);
    let config = runtime_config(environment, None);
    fixture_request(worker, config, http_request(method, target, body))
}

#[test]
fn response_encoding_matches_cloudflare() -> TestResult {
    use tokio::io::AsyncReadExt;

    let reference = Command::new("node")
        .arg(fixture_root().join("encoding-reference.mjs"))
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: Vec<serde_json::Value> = serde_json::from_slice(&reference.stdout)?;
    let directory = tempfile::tempdir()?;
    let worker = entry_worker(
        &fixture_root().join("encoding.mjs"),
        &directory.path().join("modules"),
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut failures = Vec::new();
    for (index, expected) in expected.iter().enumerate() {
        let request = http_request("GET", &format!("/?case={index}"), None);
        let config = runtime_config(BTreeMap::new(), None);
        let (_, headers, raw, _) = fixture_request(&worker, config, request)?;
        let encoding = headers
            .get("content-encoding")
            .ok_or("missing encoding")?
            .to_str()?;
        let mut decoded = Vec::new();
        let encoded =
            crate::globals::ContentDecoder::new(encoding.as_bytes())?.is_some_and(|decoder| {
                let body = Box::pin(std::io::Cursor::new(raw.clone()));
                let mut reader = crate::network::decoder::DecodedBody::new(body, decoder);
                runtime.block_on(reader.read_to_end(&mut decoded)).is_ok()
            });
        let actual = serde_json::json!({ "encoding": encoding, "encoded": encoded, "body": String::from_utf8_lossy(if encoded { &decoded } else { &raw }) });
        if actual != *expected {
            report_contract_difference(
                &format!("response encoding case {index}"),
                Some(&actual),
                expected,
            );
            failures.push(index);
        }
    }
    assert!(
        failures.is_empty(),
        "response encoding differs: {failures:?}"
    );
    Ok(())
}

#[test]
fn worker_response_rejects_header_overflow_without_panicking() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
export default { fetch() {
  return new Response(null, { headers: Array.from({ length: 25000 }, (_, index) => [`x-${index}`, "value"]) });
} };
"#,
        directory.path(),
    )?;
    let Err(error) = request(&worker, "overflow") else {
        return Err("header limit must be reported as a request error".into());
    };
    assert!(error.to_string().contains("max size reached"), "{error}");
    Ok(())
}

#[test]
fn worker_response_preserves_duplicate_headers() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
export default { fetch() {
  return new Response("ok", { status: 201, statusText: "Created Here", headers: [
    ["set-cookie", "first=1; Path=/"], ["set-cookie", "second=2; Path=/"],
    ["content-type", "text/plain"]
  ] });
} };
"#,
        directory.path(),
    )?;
    let (status, headers, body, status_text) = fixture_request(
        &worker,
        runtime_config(BTreeMap::new(), None),
        http_request("GET", "/", None),
    )?;
    assert_eq!(status, 201);
    assert_eq!(status_text, "Created Here");
    assert_eq!(body, b"ok");
    assert_eq!(
        headers.len(),
        3,
        "both Set-Cookie values must reach the gateway"
    );
    assert_eq!(
        headers.get_all("set-cookie").iter().collect::<Vec<_>>(),
        ["first=1; Path=/", "second=2; Path=/"]
    );
    Ok(())
}

/// The response of `worker`, with `config`, to `request`.
fn fixture_request(
    worker: &WorkerBundle,
    config: RuntimeConfig,
    request: HttpRequest,
) -> TestResult<(u16, HeaderMap, Vec<u8>, String)> {
    let dispatcher = Dispatcher::new(worker.clone(), config);
    let (sender, receiver) = flume::bounded(1);
    let job = Job {
        request,
        response: sender,
        websocket: None,
    };
    // The worker runs on a blocking Tokio thread, as the gateway runs it, so
    // streamed bodies can be consumed here.
    let tokio = tokio::runtime::Runtime::new()?;
    let handle = tokio.spawn_blocking(move || {
        dispatcher
            .handle(job, &CancellationToken::new())
            .map_err(|error| error.to_string())
    });
    let response = match receiver.recv_timeout(Duration::from_secs(30)) {
        Err(flume::RecvTimeoutError::Disconnected) => {
            tokio.block_on(handle)??;
            return Err("the worker sent no response".into());
        }
        response => response?,
    };
    let JobResponse::Http(response) = response else {
        return Err("unexpected websocket".into());
    };
    let body = match response.body {
        HttpBody::Buffered(body) => body,
        HttpBody::Stream(stream) => {
            let mut collected = Vec::new();
            while let Ok(chunk) = stream.recv() {
                collected.extend(chunk.map_err(io::Error::other)?);
            }
            collected
        }
    };
    tokio.block_on(handle)??;
    Ok((
        response.status,
        response.headers,
        body,
        response.status_text,
    ))
}

fn http_request(method: &str, target: &str, body: Option<Vec<u8>>) -> HttpRequest {
    let mut headers = HeaderMap::new();
    if body.is_some() {
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
    }
    HttpRequest {
        persistent: true,
        method: method.to_owned(),
        target: target.to_owned(),
        url: format!("https://app.tokamak.local{target}"),
        headers,
        body,
    }
}

/// One asset configuration's answers in `assets-reference.mjs`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssetsContract {
    html_handling: HtmlHandling,
    not_found_handling: NotFoundHandling,
    /// Each request's method, path and whether it is a navigation.
    requests: Vec<(String, String, bool)>,
    router: Vec<serde_json::Value>,
    binding: Vec<serde_json::Value>,
}

#[test]
fn assets_match_cloudflare() -> TestResult {
    let contracts: Vec<AssetsContract> =
        serde_json::from_value(node_reference("assets-reference.mjs")?)?;
    let fixture = fixture_root().join("assets");
    let directory = tempfile::tempdir()?;
    let worker = entry_worker(&fixture.join("worker.mjs"), directory.path())?;
    let public = fixture.join("public");
    let mut files = BTreeMap::new();
    for file in walkdir::WalkDir::new(&public) {
        let file = file?;
        if file.file_type().is_file() {
            let name = file.path().strip_prefix(&public)?.to_string_lossy();
            files.insert(name.replace('\\', "/"), "text/plain".to_owned());
        }
    }
    for contract in contracts {
        let manifest = AssetManifest {
            binding: "STATIC".to_owned(),
            files: files.clone(),
            html_handling: contract.html_handling,
            not_found_handling: contract.not_found_handling,
        };
        let assets = Some(Arc::new(Assets::new(public.clone(), manifest)));
        let mut router = Vec::new();
        for (method, path, navigation) in &contract.requests {
            let mut request = http_request(method, path, None);
            if *navigation {
                let navigate = HeaderValue::from_static("navigate");
                request.headers.insert("sec-fetch-mode", navigate);
            }
            let config = runtime_config(BTreeMap::new(), assets.clone());
            let (status, _, body, _) = fixture_request(&worker, config, request)?;
            router.push(serde_json::json!({ "status": status, "body": String::from_utf8(body)? }));
        }
        let requests = serde_json::to_vec(&contract.requests)?;
        let request = http_request("POST", "/binding", Some(requests));
        let config = runtime_config(BTreeMap::new(), assets);
        let (_, _, binding, _) = fixture_request(&worker, config, request)?;
        let handling = (contract.html_handling, contract.not_found_handling);
        assert_eq!(router, contract.router, "{handling:?}");
        let binding: Vec<serde_json::Value> = serde_json::from_slice(&binding)?;
        assert_eq!(binding, contract.binding, "{handling:?}");
    }
    Ok(())
}

fn serve_gzip_upstream(listener: &TcpListener) -> Result<(), String> {
    let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
    let mut request = Vec::new();
    let mut byte = [0; 1];
    while !request.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        request.push(byte[0]);
    }
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(b"upstream-payload")
        .map_err(|error| error.to_string())?;
    let payload = encoder.finish().map_err(|error| error.to_string())?;
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|error| error.to_string())?;
    stream
        .write_all(&payload)
        .map_err(|error| error.to_string())
}

#[test]
fn node_net_socket_round_trip_uses_tokamak_socket_transport() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let mut request = [0; 4];
        stream
            .read_exact(&mut request)
            .map_err(|error| error.to_string())?;
        assert_eq!(&request, b"ping");
        stream
            .read_exact(&mut request)
            .map_err(|error| error.to_string())?;
        assert_eq!(&request, b"next");
        stream
            .write_all(b"pong")
            .map_err(|error| error.to_string())?;
        stream
            .read_exact(&mut request)
            .map_err(|error| error.to_string())?;
        assert_eq!(&request, b"last");
        stream
            .write_all(b"done")
            .map_err(|error| error.to_string())?;
        stream
            .shutdown(Shutdown::Write)
            .map_err(|error| error.to_string())?;
        Ok(())
    });

    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import net from "node:net";
export default { fetch() {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: "127.0.0.1", port: Number(process.env.FLAG) });
    const chunks = [];
    socket.on("connect", () => socket.write("ping", () => socket.write("next")));
    socket.on("data", chunk => {
      chunks.push(new TextDecoder().decode(chunk));
      if (chunks.join("") === "pong") socket.write("last");
    });
    socket.on("end", () => resolve(new Response(chunks.join(""))));
    socket.on("error", reject);
  });
} };
"#,
        directory.path(),
    )?;
    let body = request(&worker, &port.to_string())?;
    assert_eq!(body, b"pongdone");
    server.join().map_err(|_| "socket fixture panicked")??;
    Ok(())
}

#[test]
fn node_socket_end_preserves_the_readable_half() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .map_err(|error| error.to_string())?;
        assert_eq!(request, b"request");
        stream
            .write_all(b"response after FIN")
            .map_err(|error| error.to_string())?;
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import net from "node:net";
export default { fetch() {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: "127.0.0.1", port: Number(process.env.FLAG) });
    let body = "";
    socket.on("error", reject);
    socket.on("data", chunk => { body += chunk.toString(); });
    socket.on("close", () => resolve(new Response(body)));
    socket.end("request");
  });
} };
"#,
        directory.path(),
    )?;
    let body = request(&worker, &port.to_string())?;
    server.join().map_err(|_| "socket fixture panicked")??;
    assert_eq!(body, b"response after FIN");
    Ok(())
}

#[test]
fn node_socket_stops_reading_under_backpressure() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| error.to_string())?;
        let mut request = [0; 4];
        stream
            .read_exact(&mut request)
            .map_err(|error| error.to_string())?;
        stream
            .write_all(&vec![42; 256 * 1024])
            .map_err(|error| error.to_string())?;
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r#"
import net from "node:net";
export default { fetch() {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: "127.0.0.1", port: Number(process.env.FLAG), readableHighWaterMark: 1 });
    const events = [];
    let bytes = 0;
    let bounded = false;
    socket.on("error", reject);
    socket.on("data", chunk => { bytes += chunk.length; });
    socket.on("end", () => events.push("end"));
    socket.on("close", () => {
      events.push("close");
      resolve(Response.json({ bytes, events, bounded }));
    });
    socket.on("connect", () => {
      socket.pause();
      socket.once("readable", () => {
        const buffered = socket.readableLength;
        const read = socket.bytesRead;
        setTimeout(() => {
          bounded = buffered > 0 && socket.readableLength === buffered && socket.bytesRead === read;
          socket.resume();
        }, 10);
      });
      socket.write("ping");
    });
  });
} };
"#,
        directory.path(),
    )?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, &port.to_string())?)?;
    assert_eq!(
        actual,
        serde_json::json!({ "bytes": 256 * 1024, "events": ["end", "close"], "bounded": true })
    );
    server.join().map_err(|_| "socket fixture panicked")??;
    Ok(())
}

#[test]
fn astro_example_renders_its_home_page() -> TestResult {
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("no workspace")?
        .join("examples/astro");
    if !example.join("dist/server/entry.mjs").is_file() {
        return Err("Astro runtime fixture is missing; build it as the README's checks do".into());
    }
    let directory = tempfile::tempdir()?;
    let worker = module_worker(
        &example.join("dist/server"),
        "entry.mjs",
        &directory.path().join("modules"),
    )?;
    let (status, _, home, _) = fixture_request(
        &worker,
        runtime_config(BTreeMap::new(), None),
        http_request("GET", "/", None),
    )?;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&home));
    assert!(String::from_utf8(home)?.contains("<html"));
    Ok(())
}

#[test]
fn builtin_modules_and_request_state_are_isolated() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r"
import { env, waitUntil } from 'cloudflare:workers';
import fs from 'node:fs';
import streams from 'node:stream';
import promises from 'node:fs/promises';
import bareFs from 'fs';
import barePromises from 'fs/promises';
import bareStreams from 'stream';
import processModule from 'process';
import events from 'events';
let calls = 0;
export default { async fetch() {
  if (fs.existsSync('/tmp/previous')) throw new Error('tmp leaked');
  const stream = fs.createReadStream('/dev/null');
  if (!(stream instanceof streams.Readable)) throw new Error('stream identity');
  if (fs.promises !== promises) throw new Error('fs promises identity');
  if (bareFs !== fs || barePromises !== promises || bareStreams !== streams || processModule !== process) throw new Error('alias identity');
  for (const [name, value] of [['fs', fs], ['fs/promises', promises], ['events', events], ['stream', streams], ['process', process]]) {
    if ((await import(name)).default !== value || (await import('node:' + name)).default !== value) throw new Error('dynamic alias identity');
  }
  waitUntil(Promise.resolve().then(() => fs.writeFileSync('/tmp/previous', env.FLAG)));
  return new Response(JSON.stringify({ calls: ++calls, flag: env.FLAG }));
} };
",
        directory.path(),
    )?;
    assert_eq!(request(&worker, "first")?, br#"{"calls":1,"flag":"first"}"#);
    assert_eq!(
        request(&worker, "second")?,
        br#"{"calls":1,"flag":"second"}"#
    );
    Ok(())
}

#[test]
fn imported_wait_until_retains_pending_work() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = WorkerBundle::of_source(
        r"
import { waitUntil } from 'cloudflare:workers';
export default { async fetch() {
  waitUntil(Promise.resolve().then(() => waitUntil(new Promise(() => {}))));
  return new Response('ready');
} };
",
        directory.path(),
    )?;
    let error = request(&worker, "first")
        .err()
        .ok_or("unsettled waitUntil was discarded")?;
    assert!(error.to_string().contains("waitUntil"), "{error}");
    Ok(())
}

#[test]
fn application_imports_cannot_access_runtime_internals() -> TestResult {
    let directory = tempfile::tempdir()?;
    for name in [
        "tokamak:host",
        "./tokamak:globals/web.mjs",
        "node:unsupported",
    ] {
        let source = format!(
            "export default {{ async fetch() {{ await import('{name}'); return new Response('unexpected'); }} }};"
        );
        let worker = WorkerBundle::of_source(source.as_bytes(), directory.path())?;
        assert!(
            request(&worker, "first").is_err(),
            "private import succeeded: {name}"
        );
    }
    Ok(())
}
