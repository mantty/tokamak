use reqwest::header::{HeaderMap, HeaderValue};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool};
use std::thread;
use std::time::Duration;

use crate::dispatcher::{AssetService, execute_request};
use crate::gateway::{Job, JobResponse, Lifecycle};
use crate::quickjs::{Assets, RuntimeConfig, WorkerBundle};
use crate::transport::{HttpBody, HttpRequest};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/quickjs_runtime")
}

fn bundle_worker(entry: &Path, output: &Path) -> TestResult<WorkerBundle> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("no workspace")?;
    let host = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64/bin/esbuild",
        ("macos", "x86_64") => "darwin-x64/bin/esbuild",
        ("linux", "x86_64") => "linux-x64/bin/esbuild",
        ("windows", "x86_64") => "win32-x64/esbuild.exe",
        _ => return Err("unsupported esbuild test host".into()),
    };
    let esbuild = workspace
        .join("tools/esbuild-hosts/node_modules/@esbuild")
        .join(host);
    let result = Command::new(esbuild)
        .args([
            "--bundle",
            "--splitting",
            "--format=esm",
            "--platform=neutral",
            "--target=es2022",
            "--entry-names=entry",
            "--chunk-names=chunks/[name]-[hash]",
            "--log-level=error",
        ])
        .args(
            crate::runtime_modules::runtime_module_names()
                .into_iter()
                .map(|name| format!("--external:{name}")),
        )
        .arg(format!("--outdir={}", output.display()))
        .arg(entry)
        .output()?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned().into());
    }
    for file in walkdir::WalkDir::new(output) {
        let file = file?;
        if !file.file_type().is_file() || file.path().extension().is_none_or(|ext| ext != "js") {
            continue;
        }
        let name = file
            .path()
            .strip_prefix(output)?
            .to_str()
            .ok_or("non-UTF8 module")?
            .replace('\\', "/");
        let bytecode = crate::compile_module(&name, &fs::read(file.path())?)?;
        fs::write(output.join(format!("{name}.qjs")), bytecode)?;
    }
    Ok(WorkerBundle::from_modules("entry.js", output, output))
}

fn request(worker: &WorkerBundle, flag: &str) -> TestResult<Vec<u8>> {
    let directory = tempfile::tempdir()?;
    let config = RuntimeConfig {
        assets: None,
        cache: directory.path().join("cache"),
        environment: BTreeMap::from([("FLAG".to_owned(), serde_json::json!(flag))]),
    };
    let accepting = Arc::new(AtomicBool::new(true));
    let lifecycle = Lifecycle::new();
    let execution = lifecycle.enter(&accepting).ok_or("request rejected")?;
    let (sender, receiver) = flume::bounded(1);
    execute_request(
        worker,
        &config,
        None,
        Job {
            request: HttpRequest {
                persistent: true,
                method: "GET".to_owned(),
                target: "/".to_owned(),
                url: "https://app.tokamak.local/".to_owned(),
                headers: HeaderMap::new(),
                body: None,
            },
            response: sender,
            websocket: None,
        },
        &execution,
    )?;
    let JobResponse::Http(response) = receiver.recv()? else {
        return Err("unexpected websocket".into());
    };
    assert_eq!(response.status, 200);
    let HttpBody::Buffered(body) = response.body else {
        return Err("unexpected stream".into());
    };
    Ok(body)
}

#[test]
fn node_web_stream_adapters_close_and_flush_vectors() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("web-adapters.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("bundle"))?;
    assert_eq!(request(&worker, "enabled")?, br#"{"chunks":[1,2,3,4,3,4],"errors":["sink rejected","sink rejected"],"callbacks":["sink rejected","sink rejected","sink rejected","sink rejected"]}"#);
    Ok(())
}

#[test]
fn node_web_duplex_failure_closes_the_live_peer() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("web-duplex-failure.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("bundle"))?;
    assert_eq!(
        request(&worker, "enabled")?,
        br#"[["writable","readable"],["readable","writable"]]"#
    );
    Ok(())
}

#[test]
fn brotli_small_output_buffers_drain_without_recursion() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("brotli-small-buffers.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("bundle"))?;
    assert_eq!(request(&worker, "enabled")?, br#"{"restored":true}"#);
    Ok(())
}

#[test]
fn due_timers_preserve_order_and_microtask_checkpoints() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("timer-order.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("bundle"))?;
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
    let source = directory.path().join("socket-timer.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
        let worker = bundle_worker(
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
    let source = directory.path().join("concurrent-fetch.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
    let source = directory.path().join("startup.mjs");
    fs::write(
        &source,
        "import { Readable } from 'node:stream'; const decoder = new TextDecoder(); const value = decoder.decode(new TextEncoder().encode('ready')); const signal = AbortSignal.any([]); export default { async fetch() { const chunks = []; for await (const chunk of Readable.from([value], { signal })) chunks.push(chunk); return new Response(chunks.join('')); } };",
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    assert_eq!(request(&worker, "first")?, b"ready");
    Ok(())
}

#[test]
fn node_http_handler_uses_tokamak_request_response_boundary() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("http-handler.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let config = RuntimeConfig {
        assets: None,
        cache: directory.path().join("cache"),
        environment: BTreeMap::new(),
    };
    let accepting = Arc::new(AtomicBool::new(true));
    let lifecycle = Lifecycle::new();
    let execution = lifecycle.enter(&accepting).ok_or("request rejected")?;
    let (sender, receiver) = flume::bounded(1);
    execute_request(
        &worker,
        &config,
        None,
        Job {
            request: HttpRequest {
                persistent: true,
                method: "POST".to_owned(),
                target: "/bridge?value=1".to_owned(),
                url: "https://app.tokamak.local/bridge?value=1".to_owned(),
                headers: HeaderMap::new(),
                body: Some(b"payload".to_vec()),
            },
            response: sender,
            websocket: None,
        },
        &execution,
    )?;
    let JobResponse::Http(response) = receiver.recv()? else {
        return Err("unexpected websocket".into());
    };
    assert_eq!(response.status, 201);
    assert_eq!(
        response.headers.get("x-tokamak-boundary"),
        Some(&HeaderValue::from_static("yes"))
    );
    let HttpBody::Buffered(body) = response.body else {
        return Err("unexpected stream".into());
    };
    assert_eq!(body, b"POST /bridge?value=1 payload");
    Ok(())
}

#[test]
fn worker_entrypoint_receives_context_and_module_exports() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("entrypoint.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
fn bundled_worker_matches_cloudflare_node_compat() -> TestResult {
    let reference = Command::new("node")
        .arg(fixture_root().join("workerd-reference.mjs"))
        .output()?;
    if !reference.status.success() {
        return Err(format!(
            "workerd reference failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        )
        .into());
    }
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    let directory = tempfile::tempdir()?;
    let worker = bundle_worker(&fixture_root().join("startup.mjs"), directory.path())?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "enabled")?)?;
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
fn every_public_module_spelling_imports() -> TestResult {
    let names = serde_json::to_string(&crate::runtime_modules::runtime_module_names())?;
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("spellings.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    assert_eq!(request(&worker, "first")?, b"[]");
    Ok(())
}

#[test]
fn request_boundary_matches_cloudflare() -> TestResult {
    let reference = Command::new("node")
        .arg(fixture_root().join("boundary-reference.mjs"))
        .output()?;
    if !reference.status.success() {
        return Err(format!(
            "boundary reference failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        )
        .into());
    }
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let upstream = thread::spawn(move || serve_gzip_upstream(&listener));
    let directory = tempfile::tempdir()?;
    let worker = bundle_worker(
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
    fixture_request(worker, environment, None, method, target, body)
}

#[test]
fn response_encoding_matches_cloudflare() -> TestResult {
    use async_compression::tokio::bufread::GzipDecoder;
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
    let worker = bundle_worker(
        &fixture_root().join("encoding.mjs"),
        &directory.path().join("modules"),
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut failures = Vec::new();
    for (index, expected) in expected.iter().enumerate() {
        let (_, headers, raw, _) = fixture_request(
            &worker,
            BTreeMap::new(),
            None,
            "GET",
            &format!("/?case={index}"),
            None,
        )?;
        let encoding = headers
            .get("content-encoding")
            .ok_or("missing encoding")?
            .to_str()?;
        let mut decoded = Vec::new();
        let encoded = match encoding {
            "gzip" => runtime
                .block_on(GzipDecoder::new(raw.as_slice()).read_to_end(&mut decoded))
                .is_ok(),
            "br" => runtime
                .block_on(
                    crate::network::brotli::Decoder::new(Box::pin(std::io::Cursor::new(
                        raw.clone(),
                    )))
                    .read_to_end(&mut decoded),
                )
                .is_ok(),
            _ => false,
        };
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
    let source = directory.path().join("headers.mjs");
    fs::write(
        &source,
        r#"
export default { fetch() {
  return new Response(null, { headers: Array.from({ length: 25000 }, (_, index) => [`x-${index}`, "value"]) });
} };
"#,
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let Err(error) = request(&worker, "overflow") else {
        return Err("header limit must be reported as a request error".into());
    };
    assert!(error.to_string().contains("max size reached"), "{error}");
    Ok(())
}

#[test]
fn worker_response_preserves_duplicate_headers() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("headers.mjs");
    fs::write(
        &source,
        r#"
export default { fetch() {
  return new Response("ok", { status: 201, statusText: "Created Here", headers: [
    ["set-cookie", "first=1; Path=/"], ["set-cookie", "second=2; Path=/"],
    ["content-type", "text/plain"]
  ] });
} };
"#,
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let (status, headers, body, status_text) =
        fixture_request(&worker, BTreeMap::new(), None, "GET", "/", None)?;
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

fn fixture_request(
    worker: &WorkerBundle,
    environment: BTreeMap<String, serde_json::Value>,
    assets: Option<Assets>,
    method: &str,
    target: &str,
    body: Option<Vec<u8>>,
) -> TestResult<(u16, HeaderMap, Vec<u8>, String)> {
    let directory = tempfile::tempdir()?;
    let config = RuntimeConfig {
        assets: assets.clone(),
        cache: directory.path().join("cache"),
        environment,
    };
    let mut headers = HeaderMap::new();
    if body.is_some() {
        headers.insert(
            "content-type",
            HeaderValue::from_static("application/octet-stream"),
        );
    }
    let (sender, receiver) = flume::bounded(1);
    let job = Job {
        request: HttpRequest {
            persistent: true,
            method: method.to_owned(),
            target: target.to_owned(),
            url: format!("https://app.tokamak.local{target}"),
            headers,
            body,
        },
        response: sender,
        websocket: None,
    };
    // The worker runs on its own thread so streamed bodies can be consumed here.
    let worker = worker.clone();
    let handle = thread::spawn(move || -> Result<(), String> {
        let service = assets
            .as_ref()
            .map(AssetService::new)
            .transpose()
            .map_err(|error| error.to_string())?
            .map(Arc::new);
        // Assets are served before the worker, matching Dispatcher::handle.
        if let Some(service) = &service
            && let Some(asset) = service
                .response(&job.request)
                .map_err(|error| error.to_string())?
        {
            return job
                .response
                .send(JobResponse::Http(asset))
                .map_err(|error| error.to_string());
        }
        let accepting = Arc::new(AtomicBool::new(true));
        let lifecycle = Lifecycle::new();
        let execution = lifecycle
            .enter(&accepting)
            .ok_or("request rejected".to_owned())?;
        execute_request(&worker, &config, service.as_ref(), job, &execution)
            .map_err(|error| error.to_string())
    });
    let JobResponse::Http(response) = receiver.recv_timeout(Duration::from_secs(30))? else {
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
    handle.join().map_err(|_| "boundary worker panicked")??;
    Ok((
        response.status,
        response.headers,
        body,
        response.status_text,
    ))
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
    let source = directory.path().join("socket.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
    let source = directory.path().join("socket-half-close.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
    let source = directory.path().join("socket-backpressure.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, &port.to_string())?)?;
    assert_eq!(
        actual,
        serde_json::json!({ "bytes": 256 * 1024, "events": ["end", "close"], "bounded": true })
    );
    server.join().map_err(|_| "socket fixture panicked")??;
    Ok(())
}

#[test]
fn astro_example_renders_pages_and_serves_assets() -> TestResult {
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("no workspace")?
        .join("examples/astro");
    if !example.join("dist/server/entry.mjs").is_file() {
        return Err("Astro runtime fixture is missing; run pnpm --dir examples/astro install --frozen-lockfile and pnpm --dir examples/astro build before testing".into());
    }
    let directory = tempfile::tempdir()?;
    let worker = bundle_worker(
        &example.join("tests/startup.mjs"),
        &directory.path().join("modules"),
    )?;
    let manifest = directory.path().join("asset-manifest.json");
    let client = example.join("dist/client");
    write_asset_manifest_for(&client, &manifest)?;
    let assets = Assets {
        manifest,
        root: client,
    };
    let (status, _, home, _) = fixture_request(
        &worker,
        BTreeMap::new(),
        Some(assets.clone()),
        "GET",
        "/",
        None,
    )?;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&home));
    assert!(String::from_utf8(home)?.contains("<html"));
    // Prerendered pages and static files reach the worker through env.ASSETS.
    let (status, _, about, _) = fixture_request(
        &worker,
        BTreeMap::new(),
        Some(assets.clone()),
        "GET",
        "/about",
        None,
    )?;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&about));
    assert!(String::from_utf8(about)?.contains("About - tokamak Example"));
    let (status, _, favicon, _) = fixture_request(
        &worker,
        BTreeMap::new(),
        Some(assets),
        "GET",
        "/favicon.ico",
        None,
    )?;
    assert_eq!(status, 200);
    assert!(!favicon.is_empty());
    Ok(())
}

fn write_asset_manifest_for(client: &Path, manifest: &Path) -> TestResult {
    let mut files = serde_json::Map::new();
    for entry in walkdir::WalkDir::new(client) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(client)?
            .to_str()
            .ok_or("non-UTF8 asset path")?
            .replace('\\', "/");
        let mime = mime_guess::from_path(entry.path())
            .first_or_octet_stream()
            .to_string();
        files.insert(format!("/{relative}"), serde_json::json!(mime));
    }
    fs::write(
        manifest,
        serde_json::to_vec(&serde_json::json!({
            "files": files,
            "htmlHandling": "auto-trailing-slash",
        }))?,
    )?;
    Ok(())
}

#[test]
fn builtin_modules_and_request_state_are_isolated() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("isolation.mjs");
    fs::write(
        &source,
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
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
    let source = directory.path().join("pending.mjs");
    fs::write(
        &source,
        r"
import { waitUntil } from 'cloudflare:workers';
export default { async fetch() {
  waitUntil(Promise.resolve().then(() => waitUntil(new Promise(() => {}))));
  return new Response('ready');
} };
",
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
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
        let worker = WorkerBundle::from_bytecode(
            crate::compile_worker(source.as_bytes())?,
            directory.path(),
        );
        assert!(
            request(&worker, "first").is_err(),
            "private import succeeded: {name}"
        );
    }
    Ok(())
}
