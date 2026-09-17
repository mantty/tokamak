#![cfg(all(feature = "native", target_os = "macos"))]

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use openssl::ssl::{SslConnector, SslFiletype, SslMethod, SslStream};
use rcgen::{CertificateParams, KeyPair};
use serde_json::json;
use tokamak::compile_worker;
use tokamak::{
    Config, PackageLayout, Runtime, WorkerEnvironment, compress_worker_bundle,
    write_worker_environment,
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
fn suspended_runtime_delays_new_gateway_connections_until_resume() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) = start_packaged_runtime(
        temporary.path(),
        worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let host = HOST;
    runtime.suspend()?;
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy.set_read_timeout(Some(Duration::from_millis(100)))?;
    proxy
        .write_all(format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;

    let Err(error) = read_header_block(&mut proxy) else {
        return Err("suspended gateway responded".into());
    };
    let error = error
        .downcast_ref::<std::io::Error>()
        .ok_or("suspended gateway returned a non-IO error")?;
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ));

    proxy.set_read_timeout(Some(Duration::from_secs(2)))?;
    runtime.resume()?;
    let response = read_header_block(&mut proxy)?;
    assert!(String::from_utf8(response)?.starts_with("HTTP/1.1 200"));
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
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        websocket_worker_source(),
        &WorkerEnvironment::default(),
    )?;
    let mut tls = open_websocket(&runtime, &state)?;

    let payload: Vec<u8> = (0..=u8::MAX).cycle().take(1 << 20).collect();
    write_masked_frame(&mut tls, true, 0x2, &payload)?;
    let (opcode, echoed) = read_server_frame(&mut tls)?;
    assert_eq!(opcode, 0x2);
    assert!(echoed == payload, "echoed {} bytes", echoed.len());
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

fn start_packaged_runtime(
    temporary: &Path,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult<(Runtime, PathBuf)> {
    let app = PackageLayout::new(temporary.join("app"));
    fs::create_dir_all(app.root())?;
    write_worker_environment(&app, environment)?;
    fs::write(
        app.worker_bundle(),
        compress_worker_bundle(&compile_worker(worker)?)?,
    )?;

    let state = temporary.join("state");
    let runtime = Runtime::start(
        Config {
            app,
            state_dir: state.clone(),
            host: HOST.to_owned(),
        },
        |_| {},
    )?;
    Ok((runtime, state))
}

fn worker_source() -> &'static [u8] {
    br#"
globalThis.Request = class Request {
  constructor(url, init = {}) {
    this.url = url;
    this.method = init.method ?? "GET";
    this.headers = init.headers ?? {};
    this.body = init.body;
  }
};
const headers = new Map([["content-type", "text/plain"]]);
export default {
  fetch: async (_request, env, ctx) => {
    ctx.waitUntil(Promise.reject(new Error("background failure")));
    const valid = env.TEXT === "value" && env.JSON?.enabled === true;
    return { status: valid ? 204 : 500, headers, text: async () => "" };
  }
};
"#
}

fn websocket_worker_source() -> &'static [u8] {
    br#"
globalThis.Request = class Request {
  constructor(url, init = {}) {
    this.url = url;
    this.method = init.method ?? "GET";
    this.headers = init.headers ?? {};
  }
};
globalThis.Response = class Response {
  constructor(_body = null, init = {}) {
    this.status = init.status ?? 200;
    this.headers = new Map();
    this.webSocket = init.webSocket;
  }
  async text() { return ""; }
};
class Socket {
  constructor() {
    this.__tokamak_outbox = [];
    this.__tokamak_peer = undefined;
    this.__tokamak_listener = undefined;
    this.__tokamak_receive = (data, binary) => this.__tokamak_listener?.({ data, binary });
    this.__tokamak_close = () => {};
  }
  accept() {}
  addEventListener(name, listener) { if (name === "message") this.__tokamak_listener = listener; }
  send(data) {
    this.__tokamak_peer.__tokamak_outbox.push({ type: "message", binary: data instanceof ArrayBuffer, data });
  }
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
    server.addEventListener("message", (event) => {
      if (event.data === "fail") throw new Error("worker failure");
      server.send(event.binary ? event.data : `pong ${event.data}`);
    });
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#
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

fn open_websocket(runtime: &Runtime, state: &Path) -> TestResult<SslStream<TcpStream>> {
    let host = HOST;
    let client_certificate = state.join("client.cert.pem");
    let client_key = state.join("client.key.pem");
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy.set_read_timeout(Some(Duration::from_secs(2)))?;
    proxy
        .write_all(format!("CONNECT {host}:443 HTTP/1.1\r\nHost: {host}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;
    let proxy_response =
        read_header_block(&mut proxy).map_err(|error| format!("proxy response: {error}"))?;
    assert!(String::from_utf8(proxy_response)?.starts_with("HTTP/1.1 200"));

    let mut connector = SslConnector::builder(SslMethod::tls())?;
    connector.set_ca_file(state.join("ca.cert.pem"))?;
    connector.set_certificate_file(client_certificate, SslFiletype::PEM)?;
    connector.set_private_key_file(client_key, SslFiletype::PEM)?;
    let connector = connector.build();
    let mut tls = connector.connect(host, proxy)?;
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    tls.write_all(
        format!(
            "GET /socket HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        .as_bytes(),
    )?;
    tls.flush()?;
    let upgrade_response = read_header_block(&mut tls)
        .map_err(|error| format!("WebSocket upgrade response: {error}"))?;
    assert!(String::from_utf8(upgrade_response)?.starts_with("HTTP/1.1 101"));
    Ok(tls)
}
