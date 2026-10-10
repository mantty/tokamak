#![cfg(all(feature = "native", target_os = "macos"))]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rcgen::{CertificateParams, KeyPair};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use serde_json::json;
use tokamak::{
    Config, ModuleType, PackageLayout, Runtime, StorageBinding, WorkerEnvironment, WorkerManifest,
    write_worker, write_worker_environment,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const HOST: &str = "app.tokamak.local";

#[test]
fn starts_a_packaged_worker_with_its_declared_environment() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        worker_source(),
        &WorkerEnvironment {
            vars: BTreeMap::from([
                ("TEXT".to_owned(), json!("value")),
                ("JSON".to_owned(), json!({ "enabled": true })),
            ]),
            storage: Vec::new(),
        },
    )?;
    let host = HOST;
    let client = (state.join("client.cert.pem"), state.join("client.key.pem"));
    let foreign = write_foreign_client(temporary.path())?;
    let script = write_client_script(temporary.path())?;

    assert!(
        !connect(
            &script,
            runtime.port(),
            host,
            &state.join("ca.cert.pem"),
            None
        )?
        .status
        .success()
    );
    let output = connect(
        &script,
        runtime.port(),
        host,
        &state.join("ca.cert.pem"),
        Some(&client),
    )?;
    assert!(
        output.status.success(),
        "stdout={:?} stderr={:?}",
        output.stdout,
        output.stderr
    );
    assert!(
        !connect(
            &script,
            runtime.port(),
            host,
            &state.join("ca.cert.pem"),
            Some(&foreign)
        )?
        .status
        .success()
    );
    assert!(
        !connect(&script, runtime.port(), host, &foreign.0, Some(&client))?
            .status
            .success()
    );
    Ok(())
}

#[test]
fn keeps_the_connection_open_between_requests_until_the_client_closes_it() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        worker_source(),
        &WorkerEnvironment {
            vars: BTreeMap::from([
                ("TEXT".to_owned(), json!("value")),
                ("JSON".to_owned(), json!({ "enabled": true })),
            ]),
            storage: Vec::new(),
        },
    )?;
    let mut tls = connect_gateway(&runtime, &state)?;

    for _ in 0..2 {
        tls.write_all(format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())?;
        tls.flush()?;
        let response = String::from_utf8(read_header_block(&mut tls)?)?;
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
        assert!(response.contains("connection: keep-alive"), "{response}");
        assert!(
            response.contains("x-request-connection: Keep-Alive"),
            "{response}"
        );
    }

    tls.write_all(
        format!("GET / HTTP/1.1\r\nHost: {HOST}\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    tls.flush()?;
    let response = String::from_utf8(read_header_block(&mut tls)?)?;
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    assert!(response.contains("connection: close"), "{response}");
    let mut rest = Vec::new();
    tls.read_to_end(&mut rest)?;
    assert!(rest.is_empty());
    Ok(())
}

#[test]
fn upgrades_to_a_websocket_on_a_reused_connection() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = connect_gateway(&runtime, &state)?;
    tls.write_all(format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())?;
    tls.flush()?;
    let response = String::from_utf8(read_header_block(&mut tls)?)?;
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");

    upgrade_websocket(&mut tls)?;
    write_masked_frame(&mut tls, true, 0x1, b"ping 1")?;
    let (opcode, payload) = read_server_frame(&mut tls)?;
    assert_eq!(opcode, 0x1);
    assert_eq!(payload, b"pong ping 1");
    Ok(())
}

#[test]
fn serves_a_packaged_worker_websocket_over_the_mtls_gateway() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, false, 0x1, b"ping ")?;
    write_masked_frame(&mut tls, true, 0x0, b"42")?;
    let (opcode, payload) = read_server_frame(&mut tls)
        .map_err(|error| format!("fragmented text response: {error}"))?;
    assert_eq!(opcode, 0x1);
    assert_eq!(payload, b"pong ping 42");

    write_masked_frame(&mut tls, true, 0x2, &[1, 2, 3])?;
    let (opcode, payload) =
        read_server_frame(&mut tls).map_err(|error| format!("binary response: {error}"))?;
    assert_eq!(opcode, 0x2);
    assert_eq!(payload, [1, 2, 3]);

    write_masked_frame(&mut tls, true, 0x8, &[3, 232])?;
    let (opcode, payload) =
        read_server_frame(&mut tls).map_err(|error| format!("close response: {error}"))?;
    assert_eq!(opcode, 0x8);
    assert_eq!(payload, [3, 232]);
    Ok(())
}

#[test]
fn answers_websocket_messages_without_polling_delay() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = open_websocket(&runtime, &state)?;

    let mut roundtrips = Vec::new();
    for index in 0..20 {
        let message = format!("ping {index}");
        let started = Instant::now();
        write_masked_frame(&mut tls, true, 0x1, message.as_bytes())?;
        let (_, payload) = read_server_frame(&mut tls)?;
        roundtrips.push(started.elapsed());
        assert_eq!(payload, format!("pong {message}").as_bytes());
    }
    roundtrips.sort_unstable();
    let median = roundtrips[roundtrips.len() / 2];
    assert!(
        median < Duration::from_millis(15),
        "median WebSocket roundtrip was {median:?}"
    );
    Ok(())
}

#[test]
fn echoes_websocket_messages_larger_than_the_socket_buffers() -> TestResult {
    let payload: Vec<u8> = (0..=u8::MAX).cycle().take(1 << 20).collect();
    assert_echoes(&payload, 0x2, &payload)
}

#[test]
fn echoes_websocket_messages_with_16_bit_frame_lengths() -> TestResult {
    let message = "x".repeat(1000);
    assert_echoes(
        message.as_bytes(),
        0x1,
        format!("pong {message}").as_bytes(),
    )
}

fn assert_echoes(payload: &[u8], opcode: u8, expected: &[u8]) -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, true, opcode, payload)?;
    let (echoed_opcode, echoed) = read_server_frame(&mut tls)?;
    assert_eq!(echoed_opcode, opcode);
    assert!(echoed == expected, "echoed {} bytes", echoed.len());
    Ok(())
}

#[test]
fn closes_the_websocket_promptly_when_the_worker_fails() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, true, 0x1, b"fail")?;
    let started = Instant::now();
    assert!(read_server_frame(&mut tls).is_err());
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "connection stayed open for {elapsed:?} after the worker failed"
    );
    Ok(())
}

#[test]
fn marks_the_worker_environment_of_a_packaged_app() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) = start_packaged_runtime(
        temporary.path(),
        br#"
import { WorkerEntrypoint } from "cloudflare:workers";

export class TokamakEvents extends WorkerEntrypoint {
  dispatch() {
    return { reply: [this.env.TOKAMAK_RUNTIME, process.env.TOKAMAK_RUNTIME], listened: ["check"] };
  }
}

export default { fetch: () => new Response(null, { status: 404 }) };
"#,
        &WorkerEnvironment::default(),
    )?;

    let reply = runtime.emit("check", "{}", Duration::from_secs(5))?;

    assert_eq!(reply, r#"["true","true"]"#);
    Ok(())
}

#[test]
fn keeps_d1_data_across_runtime_restarts() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let environment = d1_environment(&[]);
    let (runtime, _) = start_packaged_runtime(temporary.path(), d1_worker_source(), &environment)?;
    runtime.emit("setup", "{}", Duration::from_secs(5))?;
    runtime.emit("hit", "{}", Duration::from_secs(5))?;
    drop(runtime);

    let (restarted, _) =
        start_packaged_runtime(temporary.path(), d1_worker_source(), &environment)?;

    assert_eq!(restarted.emit("count", "{}", Duration::from_secs(5))?, "1");
    Ok(())
}

#[test]
fn serves_concurrent_d1_writes() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), d1_worker_source(), &d1_environment(&[]))?;
    runtime.emit("setup", "{}", Duration::from_secs(5))?;

    let writes: Vec<_> = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| runtime.emit("hit", "{}", Duration::from_secs(30)).is_ok()))
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap_or(false))
            .collect()
    });

    assert!(writes.iter().all(|written| *written), "{writes:?}");

    assert_eq!(runtime.emit("count", "{}", Duration::from_secs(5))?, "16");
    Ok(())
}

#[test]
fn applies_packaged_migrations_before_the_first_query() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = PackageLayout::new(temporary.path().join("app"));
    fs::create_dir_all(app.d1_migrations("DB"))?;
    fs::write(
        app.d1_migrations("DB").join("0001_hits.sql"),
        "CREATE TABLE hits (n INTEGER);\nINSERT INTO hits VALUES (1);",
    )?;
    let (runtime, _) = start_packaged_runtime(
        temporary.path(),
        d1_worker_source(),
        &d1_environment(&["0001_hits.sql"]),
    )?;

    assert_eq!(runtime.emit("count", "{}", Duration::from_secs(5))?, "1");
    Ok(())
}

#[test]
fn keeps_r2_objects_across_restarts_and_removes_unrecorded_bodies() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let environment = r2_environment();
    let (runtime, state) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &environment)?;
    runtime.emit(
        "put",
        r#"{"key":"kept","value":"hello"}"#,
        Duration::from_secs(5),
    )?;
    drop(runtime);
    let objects = temporary.path().join("storage/r2/files/objects");
    let parts = state.join("storage/r2/files");
    fs::write(objects.join("unrecorded"), "partial")?;
    fs::write(parts.join("unrecorded"), "partial")?;

    let (restarted, _) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &environment)?;

    assert_eq!(
        restarted.emit("get", r#"{"key":"kept"}"#, Duration::from_secs(5))?,
        r#""hello""#
    );
    assert!(!objects.join("unrecorded").exists());
    assert!(!parts.join("unrecorded").exists());
    assert_eq!(fs::read_dir(&objects)?.count(), 1);
    Ok(())
}

#[test]
fn replaces_r2_objects_whole_under_concurrent_writes() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &r2_environment())?;

    let writes: Vec<_> = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..16)
            .map(|writer| {
                let runtime = &runtime;
                scope.spawn(move || {
                    let value = format!("{writer:02}").repeat(50_000);
                    let body = json!({ "key": "shared", "value": value }).to_string();
                    runtime.emit("put", &body, Duration::from_secs(30)).is_ok()
                })
            })
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap_or(false))
            .collect()
    });

    assert!(writes.iter().all(|written| *written), "{writes:?}");
    let value: String = serde_json::from_str(&runtime.emit(
        "get",
        r#"{"key":"shared"}"#,
        Duration::from_secs(5),
    )?)?;
    assert_eq!(value.len(), 100_000);
    assert!(
        value
            .as_bytes()
            .chunks(2)
            .all(|pair| pair == &value.as_bytes()[..2])
    );
    let objects = temporary.path().join("storage/r2/files/objects");
    assert_eq!(fs::read_dir(objects)?.count(), 1);
    Ok(())
}

#[test]
fn stores_request_bodies_in_r2() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &r2_environment())?;

    let size = post(&runtime, &state, "/upload", br#"{"file":"contents"}"#)?;

    assert_eq!(size, b"19");
    assert_eq!(
        runtime.emit("get", r#"{"key":"upload"}"#, Duration::from_secs(5))?,
        r#""{\"file\":\"contents\"}""#
    );
    Ok(())
}

#[test]
fn follows_cloudflare_where_local_r2_differs() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &r2_environment())?;

    let reply = runtime.emit("cloudflare", "{}", Duration::from_secs(5))?;

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply)?,
        json!({
            "storageClasses": ["InfrequentAccess", "Standard", "InfrequentAccess"],
            "listedMetadata": [null, null],
            "writeHttpMetadata": "HTTP metadata unknown for key `class`. Did you forget to add 'httpMetadata' to `include` when listing?",
            "encryptionKey": "Customer-provided encryption keys (ssecKey) are not supported: device storage is encrypted by the operating system.",
            "uploadMetadata": "createMultipartUpload: Your metadata headers exceed the maximum allowed metadata size. (10012)",
        })
    );
    Ok(())
}

#[test]
#[ignore = "needs the app tok packages from examples/astro at TOKAMAK_TEST_ASTRO_APP"]
fn serves_the_astro_example_that_tok_packages() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = PackageLayout::new(std::env::var("TOKAMAK_TEST_ASTRO_APP")?);
    let (runtime, state) = start_runtime(temporary.path(), app)?;
    // Events wait for `start`, so both are recorded once this returns.
    runtime.emit("suspend", "{}", Duration::from_secs(30))?;
    for (path, text) in [
        ("/", "<html"),
        ("/about", "About - tokamak Example"),
        ("/plugins", "Plugins - tokamak Example"),
        // Each entry begins with its time, which ends in Z.
        ("/events", "Z start in the foreground"),
        ("/events", "Z suspend"),
    ] {
        let mut tls = connect_gateway(&runtime, &state)?;
        tls.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {HOST}\r\nConnection: close\r\n\r\n").as_bytes(),
        )?;
        tls.flush()?;
        let mut response = Vec::new();
        tls.read_to_end(&mut response)?;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
        assert!(response.contains(text), "{path}: {response}");
    }
    Ok(())
}

fn start_packaged_runtime(
    temporary: &Path,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult<(Runtime, PathBuf)> {
    let app = PackageLayout::new(temporary.join("app"));
    fs::create_dir_all(app.root())?;
    write_worker_environment(&app, environment)?;
    fs::write(temporary.join("worker.mjs"), worker)?;
    write_worker(
        &app,
        temporary,
        &WorkerManifest {
            entry: "worker.mjs".to_owned(),
            modules: BTreeMap::from([("worker.mjs".to_owned(), ModuleType::EsModule)]),
        },
    )?;
    start_runtime(temporary, app)
}

/// The runtime of the packaged `app`, with its state in `temporary`.
fn start_runtime(temporary: &Path, app: PackageLayout) -> TestResult<(Runtime, PathBuf)> {
    let state = temporary.join("state");
    let runtime = Runtime::start(
        Config {
            app,
            state_dir: state.clone(),
            storage_dir: temporary.join("storage"),
            host: HOST.to_owned(),
            foreground: true,
            plugins: None,
        },
        |_| {},
    )?;
    Ok((runtime, state))
}

fn d1_environment(migrations: &[&str]) -> WorkerEnvironment {
    WorkerEnvironment {
        vars: BTreeMap::new(),
        storage: vec![StorageBinding::D1 {
            name: "DB".to_owned(),
            id: "app".to_owned(),
            migrations_table: "d1_migrations".to_owned(),
            migrations: migrations.iter().map(|name| (*name).to_owned()).collect(),
        }],
    }
}

fn r2_environment() -> WorkerEnvironment {
    WorkerEnvironment {
        vars: BTreeMap::new(),
        storage: vec![StorageBinding::R2 {
            name: "FILES".to_owned(),
            id: "files".to_owned(),
        }],
    }
}

fn r2_worker_source() -> &'static [u8] {
    br#"
import { WorkerEntrypoint } from "cloudflare:workers";

async function cloudflare(files) {
  const infrequent = await files.put("class", "x", { storageClass: "InfrequentAccess" });
  const standard = await files.put("standard", "x");
  const listed = (await files.list({ prefix: "class" })).objects[0];
  let writeHttpMetadata;
  try { listed.writeHttpMetadata(new Headers()); } catch (error) { writeHttpMetadata = error.message; }
  return {
    storageClasses: [infrequent.storageClass, standard.storageClass, (await files.head("class")).storageClass],
    listedMetadata: [listed.httpMetadata, listed.customMetadata],
    writeHttpMetadata,
    encryptionKey: await files.put("key", "x", { ssecKey: "0".repeat(64) }).catch(error => error.message),
    uploadMetadata: await files.createMultipartUpload("upload", { customMetadata: { a: "\u0100".repeat(1024) } }).catch(error => error.message),
  };
}

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name, input) {
    const files = this.env.FILES;
    const listened = ["put", "get", "cloudflare"];
    if (name === "cloudflare") return { reply: await cloudflare(files), listened };
    if (name === "put") return { reply: (await files.put(input.key, input.value)).etag, listened };
    if (name === "get") {
      const object = await files.get(input.key);
      return { reply: object === null ? "missing" : await new Response(object.body).text(), listened };
    }
    return { listened };
  }
}

export default {
  async fetch(request, env) {
    return Response.json((await env.FILES.put("upload", request.body)).size);
  },
};
"#
}

fn d1_worker_source() -> &'static [u8] {
    br#"
import { WorkerEntrypoint } from "cloudflare:workers";

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name) {
    const db = this.env.DB;
    const listened = ["setup", "hit", "count"];
    if (name === "setup") await db.exec("CREATE TABLE IF NOT EXISTS hits (n INTEGER)");
    if (name === "hit") await db.prepare("INSERT INTO hits VALUES (?)").bind(1).run();
    if (name !== "count") return { listened };
    return { reply: await db.prepare("SELECT count(*) AS count FROM hits").first("count"), listened };
  }
}

export default { fetch: () => new Response(null, { status: 404 }) };
"#
}

fn worker_source() -> &'static [u8] {
    br#"
export default {
  fetch: async (request, env, ctx) => {
    ctx.waitUntil(Promise.reject(new Error("background failure")));
    const valid = env.TEXT === "value" && env.JSON?.enabled === true;
    const connection = request.headers.get("connection") ?? "missing";
    const headers = { "content-type": "text/plain", "x-request-connection": connection };
    return new Response(null, { status: valid ? 204 : 500, headers });
  }
};
"#
}

fn websocket_worker_source() -> &'static [u8] {
    br#"
export default {
  async fetch(request) {
    if (!request.headers.has("upgrade")) return new Response(null, { status: 204 });
    const [client, server] = Object.values(new WebSocketPair());
    server.accept();
    server.addEventListener("message", (event) => {
      if (event.data === "fail") throw new Error("worker failure");
      server.send(typeof event.data === "string" ? `pong ${event.data}` : event.data);
    });
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#
}

/// The body of the Worker's 200 response to a POST of `body` to `path`.
fn post(runtime: &Runtime, state: &Path, path: &str, body: &[u8]) -> TestResult<Vec<u8>> {
    let mut tls = connect_gateway(runtime, state)?;
    write!(
        tls,
        "POST {path} HTTP/1.1\r\nHost: {HOST}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    tls.write_all(body)?;
    tls.flush()?;
    let headers = String::from_utf8(read_header_block(&mut tls)?)?;
    if !headers.starts_with("HTTP/1.1 200") {
        return Err(format!("{path} responded {headers}").into());
    }
    let mut response = Vec::new();
    tls.read_to_end(&mut response)?;
    Ok(response)
}

fn read_header_block(stream: &mut impl Read) -> TestResult<Vec<u8>> {
    let mut data = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        data.push(byte[0]);
        if data.ends_with(b"\r\n\r\n") {
            return Ok(data);
        }
    }
}

fn write_masked_frame(
    stream: &mut impl Write,
    final_frame: bool,
    opcode: u8,
    payload: &[u8],
) -> TestResult {
    let mask = [1, 2, 3, 4];
    let mut frame = vec![(if final_frame { 0x80 } else { 0 }) | opcode];
    match payload.len() {
        length @ ..=125 => frame.push(0x80 | u8::try_from(length)?),
        length @ ..=0xFFFF => {
            frame.push(0x80 | 0x7E);
            frame.extend_from_slice(&u16::try_from(length)?.to_be_bytes());
        }
        length => {
            frame.push(0x80 | 0x7F);
            frame.extend_from_slice(&u64::try_from(length)?.to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

fn read_server_frame(stream: &mut impl Read) -> TestResult<(u8, Vec<u8>)> {
    let mut header = [0; 2];
    stream.read_exact(&mut header)?;
    assert_eq!(header[0] & 0x80, 0x80);
    assert_eq!(header[1] & 0x80, 0);
    let mut length = usize::from(header[1] & 0x7f);
    if length == 126 {
        let mut value = [0; 2];
        stream.read_exact(&mut value)?;
        length = usize::from(u16::from_be_bytes(value));
    } else if length == 127 {
        let mut value = [0; 8];
        stream.read_exact(&mut value)?;
        length = usize::try_from(u64::from_be_bytes(value))?;
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    Ok((header[0] & 0x0f, payload))
}

fn write_foreign_client(directory: &Path) -> TestResult<(PathBuf, PathBuf)> {
    let key = KeyPair::generate()?;
    let certificate = CertificateParams::default().self_signed(&key)?;
    let cert_path = directory.join("foreign.cert.pem");
    let key_path = directory.join("foreign.key.pem");
    fs::write(&cert_path, certificate.pem())?;
    fs::write(&key_path, key.serialize_pem())?;
    Ok((cert_path, key_path))
}

fn write_client_script(directory: &Path) -> TestResult<PathBuf> {
    let script = directory.join("tls-client.mjs");
    fs::write(&script, include_str!("fixtures/tls-client.mjs"))?;
    Ok(script)
}

fn connect(
    script: &Path,
    port: u16,
    host: &str,
    authority: &Path,
    client: Option<&(PathBuf, PathBuf)>,
) -> TestResult<Output> {
    let mut command = Command::new("node");
    command
        .arg(script)
        .arg(port.to_string())
        .arg(host)
        .arg(authority);
    if let Some((certificate, key)) = client {
        command.arg(certificate).arg(key);
    } else {
        command.args(["", ""]);
    }
    Ok(command.output()?)
}

fn connect_gateway(
    runtime: &Runtime,
    state: &Path,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy.set_read_timeout(Some(Duration::from_secs(2)))?;
    proxy
        .write_all(format!("CONNECT {HOST}:443 HTTP/1.1\r\nHost: {HOST}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;
    let proxy_response =
        read_header_block(&mut proxy).map_err(|error| format!("proxy response: {error}"))?;
    assert!(String::from_utf8(proxy_response)?.starts_with("HTTP/1.1 200"));
    connect_tls(state, HOST, proxy)
}

fn open_websocket(
    runtime: &Runtime,
    state: &Path,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut tls = connect_gateway(runtime, state)?;
    upgrade_websocket(&mut tls)?;
    Ok(tls)
}

fn upgrade_websocket(tls: &mut StreamOwned<ClientConnection, TcpStream>) -> TestResult {
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    tls.write_all(
        format!(
            "GET /socket HTTP/1.1\r\nHost: {HOST}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .as_bytes(),
    )?;
    tls.flush()?;
    let upgrade_response =
        read_header_block(tls).map_err(|error| format!("WebSocket upgrade response: {error}"))?;
    assert!(String::from_utf8(upgrade_response)?.starts_with("HTTP/1.1 101"));
    Ok(())
}

fn connect_tls(
    state: &Path,
    host: &str,
    stream: TcpStream,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut roots = RootCertStore::empty();
    for certificate in CertificateDer::pem_file_iter(state.join("ca.cert.pem"))? {
        roots.add(certificate?)?;
    }
    let chain = CertificateDer::pem_file_iter(state.join("client.cert.pem"))?
        .collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_file(state.join("client.key.pem"))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)?;
    let connection =
        ClientConnection::new(Arc::new(config), ServerName::try_from(host.to_owned())?)?;
    let mut tls = StreamOwned::new(connection, stream);
    while tls.conn.is_handshaking() {
        tls.conn.complete_io(&mut tls.sock)?;
    }
    Ok(tls)
}
