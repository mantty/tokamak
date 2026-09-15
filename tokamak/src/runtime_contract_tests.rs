use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool, mpsc};
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
    let (sender, receiver) = mpsc::sync_channel(1);
    execute_request(
        worker,
        &config,
        None,
        Job {
            request: HttpRequest {
                method: "GET".to_owned(),
                target: "/".to_owned(),
                url: "https://app.tokamak.local/".to_owned(),
                headers: BTreeMap::new(),
                body: None,
            },
            response: sender,
            websocket: None,
        },
        &execution,
        &accepting,
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
fn packaged_globals_precede_application_modules() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("startup.mjs");
    fs::write(
        &source,
        "const decoder = new TextDecoder(); const value = decoder.decode(new TextEncoder().encode('ready')); export default { async fetch() { return new Response(value); } };",
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
    let (sender, receiver) = mpsc::sync_channel(1);
    execute_request(
        &worker,
        &config,
        None,
        Job {
            request: HttpRequest {
                method: "POST".to_owned(),
                target: "/bridge?value=1".to_owned(),
                url: "https://app.tokamak.local/bridge?value=1".to_owned(),
                headers: BTreeMap::new(),
                body: Some(b"payload".to_vec()),
            },
            response: sender,
            websocket: None,
        },
        &execution,
        &accepting,
    )?;
    let JobResponse::Http(response) = receiver.recv()? else {
        return Err("unexpected websocket".into());
    };
    assert_eq!(response.status, 201);
    assert_eq!(
        response.headers.get("x-tokamak-boundary"),
        Some(&"yes".to_owned())
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
    assert_eq!(response["contextExports"], serde_json::json!(["default", "named"]));
    assert_eq!(response["importedExports"], serde_json::json!(["default", "named"]));
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
    let (Some(actual), Some(expected)) = (actual.as_object(), expected.as_object()) else {
        eprintln!("Cloudflare contract: {name}: actual={actual} expected={expected}");
        return;
    };
    let extra_keys = actual.keys().filter(|key| !expected.contains_key(*key));
    for key in expected.keys().chain(extra_keys) {
        let (actual, expected) = (actual.get(key), expected.get(key));
        if actual != expected {
            let null = serde_json::Value::Null;
            eprintln!(
                "Cloudflare contract: {name}.{key}: actual={} expected={}",
                actual.unwrap_or(&null),
                expected.unwrap_or(&null),
            );
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
) -> TestResult<(u16, BTreeMap<String, String>, Vec<u8>)> {
    let environment = BTreeMap::from([(
        "UPSTREAM_PORT".to_owned(),
        serde_json::json!(upstream_port.to_string()),
    )]);
    fixture_request(worker, environment, None, method, target, body)
}

fn fixture_request(
    worker: &WorkerBundle,
    environment: BTreeMap<String, serde_json::Value>,
    assets: Option<Assets>,
    method: &str,
    target: &str,
    body: Option<Vec<u8>>,
) -> TestResult<(u16, BTreeMap<String, String>, Vec<u8>)> {
    let directory = tempfile::tempdir()?;
    let config = RuntimeConfig {
        assets: assets.clone(),
        cache: directory.path().join("cache"),
        environment,
    };
    let mut headers = BTreeMap::new();
    if body.is_some() {
        headers.insert(
            "content-type".to_owned(),
            "application/octet-stream".to_owned(),
        );
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let job = Job {
        request: HttpRequest {
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
            .map(|assets| AssetService::new(assets))
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
        execute_request(
            &worker,
            &config,
            service.as_ref(),
            job,
            &execution,
            &accepting,
        )
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
    Ok((response.status, response.headers, body))
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
    let mut encoder =
        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
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
        let mut request = [0; 4];
        stream
            .read_exact(&mut request)
            .map_err(|error| error.to_string())?;
        assert_eq!(&request, b"ping");
        stream
            .write_all(b"pong")
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
    socket.on("connect", () => socket.write("ping"));
    socket.on("data", chunk => chunks.push(new TextDecoder().decode(chunk)));
    socket.on("end", () => resolve(new Response(chunks.join(""))));
    socket.on("error", reject);
  });
} };
"#,
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let body = request(&worker, &port.to_string())?;
    assert_eq!(body, b"pong");
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
        eprintln!("skipping: examples/astro/dist is not built (run pnpm build there)");
        return Ok(());
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
    let (status, _, home) = fixture_request(
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
    let (status, _, about) = fixture_request(
        &worker,
        BTreeMap::new(),
        Some(assets.clone()),
        "GET",
        "/about",
        None,
    )?;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&about));
    assert!(String::from_utf8(about)?.contains("About - tokamak Example"));
    let (status, _, favicon) =
        fixture_request(&worker, BTreeMap::new(), Some(assets), "GET", "/favicon.ico", None)?;
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
