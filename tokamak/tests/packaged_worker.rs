#![cfg(all(feature = "native", target_os = "macos"))]

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use rcgen::{CertificateParams, KeyPair};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::ServerName;
use serde_json::json;
use tokamak::{
    AssetManifest, Config, Event, HtmlHandling, ModuleType, NotFoundHandling, PackageLayout,
    Runtime, StorageBinding, WorkerEnvironment, WorkerManifest, write_asset_manifest, write_worker,
    write_worker_environment,
};

use common::{TestResult, connect_tls, read_header_block, reset_connection, write_masked_frame};

const HOST: &str = "app.tokamak.local";
const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn starts_a_packaged_worker_with_its_declared_environment() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        worker_source(),
        &WorkerEnvironment {
            services: BTreeMap::new(),
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
    let authority = state.join("ca.cert.pem");

    for (authority, client, accepted) in [
        (&authority, None, false),
        (&authority, Some(&client), true),
        (&authority, Some(&foreign), false),
        (&foreign.0, Some(&client), false),
    ] {
        let output = connect(&script, runtime.port(), host, authority, client)?;
        assert_eq!(
            output.status.success(),
            accepted,
            "authority={authority:?} client={client:?} stdout={:?} stderr={:?}",
            output.stdout,
            output.stderr
        );
    }
    Ok(())
}

#[test]
fn keeps_the_connection_open_between_requests_until_the_client_closes_it() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        temporary.path(),
        worker_source(),
        &WorkerEnvironment {
            services: BTreeMap::new(),
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
    assert_eq!(read_server_frame(&mut tls)?, (0x1, b"pong ping 1".to_vec()));
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
    let (runtime, state, failures) = start_reporting_runtime(
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
    // The Worker's error, and the session it left open.
    let reported = [
        failures.recv_timeout(TIMEOUT)?,
        failures.recv_timeout(TIMEOUT)?,
    ];
    assert!(
        reported
            .iter()
            .any(|message| message.contains("worker failure")),
        "{reported:?}"
    );
    assert!(
        reported
            .iter()
            .any(|message| message == "WebSocket worker closed"),
        "{reported:?}"
    );
    Ok(())
}

#[test]
fn drops_what_the_worker_sends_after_the_client_leaves() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let worker = br#"
export default {
  async fetch() {
    const [client, server] = Object.values(new WebSocketPair());
    server.accept();
    server.addEventListener("message", () => {
      const until = Date.now() + 300;
      while (Date.now() < until) {}
      server.send("after the client left");
    });
    return new Response(null, { status: 101, webSocket: client });
  }
};
"#;
    let (runtime, state, failures) =
        start_reporting_runtime(temporary.path(), worker, &WorkerEnvironment::default())?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, true, 0x1, b"slow")?;
    drop(tls);
    let failure = failures.recv_timeout(Duration::from_secs(2));
    assert!(failure.is_err(), "reported {failure:?}");
    Ok(())
}

#[test]
fn tells_the_worker_when_the_client_closes_its_websocket() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state, failures) = start_close_runtime(temporary.path())?;
    let mut tls = open_websocket(&runtime, &state)?;

    // 1001, "away"
    let close = [3, 233, b'a', b'w', b'a', b'y'];
    write_masked_frame(&mut tls, true, 0x8, &close)?;
    assert_eq!(read_server_frame(&mut tls)?, (0x8, close.to_vec()));
    assert_eq!(recorded(&runtime, "close")?, json!([1001, "away", true]));
    assert_no_failures(&failures);
    Ok(())
}

#[test]
fn tells_the_worker_when_the_client_drops_its_websocket() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state, failures) = start_close_runtime(temporary.path())?;
    let tls = open_websocket(&runtime, &state)?;

    // Closes the connection without a close frame or a TLS close_notify.
    drop(tls);
    assert_eq!(
        recorded(&runtime, "close")?,
        json!([
            1006,
            "WebSocket disconnected without sending Close frame.",
            false
        ])
    );
    assert_no_failures(&failures);
    Ok(())
}

#[test]
fn tells_the_worker_how_the_client_answers_its_close() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state, failures) = start_close_runtime(temporary.path())?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, true, 0x1, b"close")?;
    // 4000, "bye"
    let close = [15, 160, b'b', b'y', b'e'];
    assert_eq!(read_server_frame(&mut tls)?, (0x8, close.to_vec()));
    write_masked_frame(&mut tls, true, 0x8, &close)?;
    assert_eq!(recorded(&runtime, "close")?, json!([4000, "bye", true]));
    // RFC 6455 allows each end one close frame.
    assert!(read_server_frame(&mut tls).is_err());
    assert_no_failures(&failures);
    Ok(())
}

#[test]
fn keeps_the_worker_websocket_closing_until_the_client_answers() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state, failures) = start_close_runtime(temporary.path())?;
    let mut tls = open_websocket(&runtime, &state)?;

    write_masked_frame(&mut tls, true, 0x1, b"close")?;
    assert_eq!(read_server_frame(&mut tls)?.0, 0x8);
    write_masked_frame(&mut tls, true, 0x1, b"late")?;
    // CLOSING, and `send` throws, as in workerd.
    assert_eq!(recorded(&runtime, "late")?, json!(["late", 2, false]));
    write_masked_frame(&mut tls, true, 0x8, &[15, 160])?;
    assert_eq!(recorded(&runtime, "close")?, json!([4000, "", true]));
    assert_no_failures(&failures);
    Ok(())
}

#[test]
fn ends_without_a_failure_when_the_client_leaves_mid_request() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, state, failures) = start_close_runtime(temporary.path())?;
    let mut tls = connect_gateway(&runtime, &state)?;

    tls.write_all(format!("GET / HTTP/1.1\r\nHost: {HOST}\r\n").as_bytes())?;
    tls.flush()?;
    tls.sock.shutdown(Shutdown::Write)?;
    // The gateway closes the connection once it reads that the client left.
    let _ = tls.read_to_end(&mut Vec::new());
    assert_no_failures(&failures);
    Ok(())
}

struct CancellablePlugins(mpsc::Sender<u64>, mpsc::Sender<u64>);

impl tokamak::PluginHandler for CancellablePlugins {
    fn call(&self, id: u64, _: &str, _: &str, _: &[u8]) {
        let _ = self.0.send(id);
    }
    fn call_with_body(&self, id: u64, _: &str, _: &str, _: &[u8]) {
        let _ = self.0.send(id);
    }
    fn chunk(&self, _: u64, _: &tokamak::PluginReply) {}
    fn subscribe(&self, id: u64, _: &str, _: &str, _: &[u8]) {
        let _ = self.0.send(id);
    }
    fn cancel(&self, id: u64) {
        let _ = self.1.send(id);
    }
}

fn cancellable_package(temporary: &Path) -> TestResult<PackageLayout> {
    let app = PackageLayout::new(temporary.join("app"));
    package_worker(
        temporary,
        &app.tokamak_worker(),
        br"const nativeCall = async (ctx, plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await ctx[Symbol.for('tokamak.native')].call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));
const nativeListen = (ctx, plugin, method, value, next, error) => ctx[Symbol.for('tokamak.native')].listen(plugin, method, new TextEncoder().encode(JSON.stringify(value)), { next: payload => next(JSON.parse(new TextDecoder().decode(payload))), error });

import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async fetch(request) { return new Response(await this.env.SELF.hold(new URL(request.url).pathname)); }
  async hold(path) {
    const ctx = this.ctx;
    nativeListen(ctx, 'gate', 'watch', null, () => {}, () => {});
    if (path === '/stream') return new ReadableStream({ pull() { return nativeCall(ctx, 'gate', 'hold'); } });
    return await nativeCall(ctx, 'gate', 'hold');
  }
}
",
        &WorkerEnvironment {
            services: BTreeMap::from([(
                "SELF".into(),
                tokamak::ServiceBinding {
                    service: "tokamak".into(),
                    entrypoint: "default".into(),
                },
            )]),
            ..WorkerEnvironment::default()
        },
    )?;
    package_worker(
        temporary,
        &app,
        b"export default { fetch: (request, env) => env.TOKAMAK.fetch(request) };",
        &WorkerEnvironment {
            services: BTreeMap::from([(
                "TOKAMAK".into(),
                tokamak::ServiceBinding {
                    service: "tokamak".into(),
                    entrypoint: "default".into(),
                },
            )]),
            ..WorkerEnvironment::default()
        },
    )?;
    Ok(app)
}

#[test]
fn cancels_service_calls_and_streams_when_the_http_client_disconnects() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = cancellable_package(temporary.path())?;
    let (subscribed, subscriptions) = mpsc::channel();
    let (removed, removals) = mpsc::channel();
    let (failed, failures) = mpsc::channel();
    let (runtime, state) = start_runtime(
        temporary.path(),
        app,
        Some(Arc::new(CancellablePlugins(subscribed, removed))),
        move |event| {
            if let Event::RequestFailed { message } = event {
                let _ = failed.send(message);
            }
        },
    )?;

    for path in ["/", "/stream", "/pipelined"] {
        let mut tls = connect_gateway(&runtime, &state)?;
        tls.write_all(format!("GET {path} HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())?;
        tls.flush()?;
        let mut ids = [
            subscriptions.recv_timeout(TIMEOUT)?,
            subscriptions.recv_timeout(TIMEOUT)?,
        ];
        ids.sort_unstable();
        if path == "/stream" {
            assert!(String::from_utf8(read_header_block(&mut tls)?)?.starts_with("HTTP/1.1 200"));
        }
        if path == "/pipelined" {
            tls.write_all(format!("GET /next HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())?;
            tls.flush()?;
        }
        reset_connection(&tls.sock)?;
        drop(tls);
        let mut cancelled = [
            removals.recv_timeout(TIMEOUT)?,
            removals.recv_timeout(TIMEOUT)?,
        ];
        cancelled.sort_unstable();
        assert_eq!(cancelled, ids);
        assert_no_failures(&failures);
    }
    Ok(())
}

#[test]
fn preserves_pipelined_requests_while_waiting_for_the_worker() -> TestResult {
    let directory = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(directory.path(), br"
export default {
  async fetch(request) {
    await new Promise(resolve => setTimeout(resolve, 200));
    return new Response(null, { status: 204, headers: { 'x-path': new URL(request.url).pathname } });
  }
};", &WorkerEnvironment::default())?;
    let mut tls = connect_gateway(&runtime, &state)?;
    tls.write_all(format!("GET /first HTTP/1.1\r\nHost: {HOST}\r\n\r\nGET /second HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes())?;
    tls.flush()?;
    for path in ["/first", "/second"] {
        let response = String::from_utf8(read_header_block(&mut tls)?)?;
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
        assert!(response.contains(&format!("x-path: {path}")), "{response}");
    }
    Ok(())
}

#[test]
fn ends_without_a_failure_when_the_client_rejects_the_certificate() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _, failures) = start_close_runtime(temporary.path())?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let mut connection = ClientConnection::new(Arc::new(config), ServerName::try_from(HOST)?)?;

    // The client trusts no certificate authority, so it aborts the handshake.
    let handshake = connection.complete_io(&mut open_tunnel(&runtime)?);
    assert!(handshake.is_err(), "the client trusted the certificate");
    assert_no_failures(&failures);
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
    const reply = JSON.stringify([this.env.TOKAMAK_RUNTIME, process.env.TOKAMAK_RUNTIME]);
    return { reply: new TextEncoder().encode(reply), listened: ["check"] };
  }
}

export default { fetch: () => new Response(null, { status: 404 }) };
"#,
        &WorkerEnvironment::default(),
    )?;

    let reply = emit(&runtime, "check", "{}", Duration::from_secs(5))?;

    assert_eq!(reply, json!(["true", "true"]));
    Ok(())
}

#[test]
fn keeps_d1_data_across_runtime_restarts() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let environment = d1_environment(&[]);
    let (runtime, _) = start_packaged_runtime(temporary.path(), d1_worker_source(), &environment)?;
    emit(&runtime, "setup", "{}", Duration::from_secs(5))?;
    emit(&runtime, "hit", "{}", Duration::from_secs(5))?;
    drop(runtime);

    let (restarted, _) =
        start_packaged_runtime(temporary.path(), d1_worker_source(), &environment)?;

    assert_eq!(
        emit(&restarted, "count", "{}", Duration::from_secs(5))?,
        json!(1)
    );
    Ok(())
}

#[test]
fn serves_concurrent_d1_writes() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), d1_worker_source(), &d1_environment(&[]))?;
    emit(&runtime, "setup", "{}", Duration::from_secs(5))?;

    let writes: Vec<_> = std::thread::scope(|scope| {
        let writers: Vec<_> = (0..16)
            .map(|_| scope.spawn(|| emit(&runtime, "hit", "{}", Duration::from_secs(30)).is_ok()))
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap_or(false))
            .collect()
    });

    assert!(writes.iter().all(|written| *written), "{writes:?}");

    assert_eq!(
        emit(&runtime, "count", "{}", Duration::from_secs(5))?,
        json!(16)
    );
    Ok(())
}

#[test]
fn applies_packaged_migrations_before_the_first_query() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = PackageLayout::new(temporary.path().join("app"));
    for package in [&app, &app.tokamak_worker()] {
        fs::create_dir_all(package.d1_migrations("DB"))?;
        fs::write(
            package.d1_migrations("DB").join("0001_hits.sql"),
            "CREATE TABLE hits (n INTEGER);\nINSERT INTO hits VALUES (1);",
        )?;
    }
    let (runtime, _) = start_packaged_runtime(
        temporary.path(),
        d1_worker_source(),
        &d1_environment(&["0001_hits.sql"]),
    )?;

    assert_eq!(
        emit(&runtime, "count", "{}", Duration::from_secs(5))?,
        json!(1)
    );
    Ok(())
}

#[test]
fn keeps_r2_objects_across_restarts_and_removes_unrecorded_bodies() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let environment = r2_environment();
    let (runtime, state) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &environment)?;
    emit(
        &runtime,
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
        emit(
            &restarted,
            "get",
            r#"{"key":"kept"}"#,
            Duration::from_secs(5)
        )?,
        json!("hello")
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
                    emit(runtime, "put", &body, Duration::from_secs(30)).is_ok()
                })
            })
            .collect();
        writers
            .into_iter()
            .map(|writer| writer.join().unwrap_or(false))
            .collect()
    });

    assert!(writes.iter().all(|written| *written), "{writes:?}");
    let value: String = serde_json::from_value(emit(
        &runtime,
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
        emit(
            &runtime,
            "get",
            r#"{"key":"upload"}"#,
            Duration::from_secs(5)
        )?,
        json!(r#"{"file":"contents"}"#)
    );
    Ok(())
}

#[test]
fn follows_cloudflare_where_local_r2_differs() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), r2_worker_source(), &r2_environment())?;

    let reply = emit(&runtime, "cloudflare", "{}", Duration::from_secs(5))?;

    assert_eq!(
        reply,
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
    let example = start_astro_example(temporary.path(), "granted")?;
    // Events wait for `start`, so each is recorded once this returns.
    example
        .runtime
        .emit("suspend", b"{}", Duration::from_secs(30))?;

    assert_no_failures(&example.failures);
    for (path, text) in [
        ("/", "<html"),
        ("/about", "About - tokamak Example"),
        ("/plugins", "Plugins - tokamak Example"),
        // Each entry begins with its time, which ends in Z.
        ("/events", "Z start in the foreground"),
        ("/events", "Z location failed: TimeoutError"),
        ("/events", "Z suspend"),
    ] {
        let mut tls = connect_gateway(&example.runtime, &example.state)?;
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

#[test]
#[ignore = "needs the app tok packages from examples/astro at TOKAMAK_TEST_ASTRO_APP"]
fn starts_the_astro_example_without_asking_for_location() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let example = start_astro_example(temporary.path(), "prompt")?;
    example
        .runtime
        .emit("suspend", b"{}", Duration::from_secs(30))?;

    assert_no_failures(&example.failures);
    let called: Vec<String> = example.called.try_iter().collect();
    assert!(
        called
            .iter()
            .any(|operation| operation == "location.permission"),
        "{called:?}"
    );
    assert!(
        !called
            .iter()
            .any(|operation| operation == "location.requestPermission"),
        "{called:?}"
    );
    Ok(())
}

#[test]
fn fetches_what_the_app_serves_at_a_path() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = PackageLayout::new(temporary.path().join("app"));
    fs::create_dir_all(app.assets().join("sounds"))?;
    fs::write(app.assets().join("sounds/chime.caf"), b"chime")?;
    write_asset_manifest(
        &app,
        &AssetManifest {
            binding: "ASSETS".to_owned(),
            files: BTreeMap::from([("sounds/chime.caf".to_owned(), "audio/x-caf".to_owned())]),
            html_handling: HtmlHandling::default(),
            not_found_handling: NotFoundHandling::default(),
        },
    )?;
    let worker = br#"export default {
      fetch: (request) => new URL(request.url).pathname === "/generated"
        ? new Response("generated")
        : new Response("missing", { status: 404 }),
    };"#;
    let (runtime, _) =
        start_packaged_runtime(temporary.path(), worker, &WorkerEnvironment::default())?;

    assert_eq!(runtime.fetch("/sounds/chime.caf", TIMEOUT)?, b"chime");
    assert_eq!(runtime.fetch("/generated", TIMEOUT)?, b"generated");
    let Err(missing) = runtime.fetch("/missing", TIMEOUT) else {
        return Err("fetched a missing path".into());
    };
    assert_eq!(missing.to_string(), "GET /missing answered 404 Not Found");
    let Err(relative) = runtime.fetch("sounds/chime.caf", TIMEOUT) else {
        return Err("fetched a URL".into());
    };
    assert_eq!(
        relative.to_string(),
        "sounds/chime.caf is not a path the app serves"
    );
    Ok(())
}

fn start_packaged_runtime(
    temporary: &Path,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult<(Runtime, PathBuf)> {
    start_runtime(
        temporary,
        package(temporary, worker, environment)?,
        None,
        |_| {},
    )
}

/// `worker`, with `environment`, packaged in `temporary` as both the main
/// Worker and the Tokamak worker, which receives the events tests emit.
fn package(
    temporary: &Path,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult<PackageLayout> {
    let app = PackageLayout::new(temporary.join("app"));
    for package in [&app, &app.tokamak_worker()] {
        package_worker(temporary, package, worker, environment)?;
    }
    Ok(app)
}

/// Writes `worker`, with `environment`, as the package `layout`.
fn package_worker(
    temporary: &Path,
    layout: &PackageLayout,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult {
    fs::create_dir_all(layout.root())?;
    write_worker_environment(layout, environment)?;
    fs::write(temporary.join("worker.mjs"), worker)?;
    write_worker(
        layout,
        temporary,
        &WorkerManifest {
            entry: "worker.mjs".to_owned(),
            modules: BTreeMap::from([("worker.mjs".to_owned(), ModuleType::EsModule)]),
        },
    )?;
    Ok(())
}

/// The JSON reply to the event `name` with the JSON `event`, or null without one.
fn emit(
    runtime: &Runtime,
    name: &str,
    event: &str,
    timeout: Duration,
) -> TestResult<serde_json::Value> {
    let reply = runtime.emit(name, event.as_bytes(), timeout)?;
    if reply.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_slice(&reply)?)
}

/// The runtime of the packaged `app`, with its state in `temporary` and the
/// shell's `plugins`, which tells `listener` its events.
fn start_runtime(
    temporary: &Path,
    app: PackageLayout,
    plugins: Option<Arc<dyn tokamak::PluginHandler>>,
    listener: impl Fn(Event) + Send + Sync + 'static,
) -> TestResult<(Runtime, PathBuf)> {
    let state = temporary.join("state");
    let runtime = Runtime::start(
        Config {
            app,
            state_dir: state.clone(),
            storage_dir: temporary.join("storage"),
            host: HOST.to_owned(),
            foreground: true,
            plugins,
        },
        listener,
    )?;
    Ok((runtime, state))
}

/// Shell plugins that pass each call's ID and `plugin.method` to the first
/// channel, and its `plugin.method` to the second as the Worker calls it.
struct PluginCalls(mpsc::Sender<(u64, String)>, mpsc::Sender<String>);

impl tokamak::PluginHandler for PluginCalls {
    fn call(&self, id: u64, plugin: &str, method: &str, _: &[u8]) {
        let operation = format!("{plugin}.{method}");
        let _ = self.1.send(operation.clone());
        let _ = self.0.send((id, operation));
    }
    fn call_with_body(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        self.call(id, plugin, method, arguments);
    }
    fn chunk(&self, _: u64, _: &tokamak::PluginReply) {}
    fn subscribe(&self, _: u64, _: &str, _: &str, _: &[u8]) {}
    fn cancel(&self, _: u64) {}
}

/// The running Astro example, its state directory, the failures it reports,
/// and the plugin operations its Worker called.
struct AstroExample {
    runtime: Arc<Runtime>,
    state: PathBuf,
    failures: Receiver<String>,
    called: Receiver<String>,
}

/// The Astro example at `TOKAMAK_TEST_ASTRO_APP`, with its state in
/// `temporary`, on a device whose location permission is `in_use`.
fn start_astro_example(temporary: &Path, in_use: &str) -> TestResult<AstroExample> {
    let (calls, received) = mpsc::channel();
    let (operations, called) = mpsc::channel();
    let (failed, failures) = mpsc::channel();
    let (runtime, state) = start_runtime(
        temporary,
        PackageLayout::new(std::env::var("TOKAMAK_TEST_ASTRO_APP")?),
        Some(Arc::new(PluginCalls(calls, operations))),
        move |event| {
            if let Event::Failed { message } = event {
                let _ = failed.send(message);
            }
        },
    )?;
    let runtime = Arc::new(runtime);
    answer_as_a_device(&runtime, received, in_use);
    Ok(AstroExample {
        runtime,
        state,
        failures,
        called,
    })
}

/// Answers `calls` as a device whose location permission is `in_use` and
/// which knows no recent position, where `location.requestPermission` and
/// `location.currentPosition` never reply.
fn answer_as_a_device(runtime: &Arc<Runtime>, calls: Receiver<(u64, String)>, in_use: &str) {
    let runtime = Arc::downgrade(runtime);
    let permission = format!(r#"{{"inUse":"{in_use}"}}"#);
    std::thread::spawn(move || {
        for (id, operation) in calls {
            let payload = match operation.as_str() {
                "location.requestPermission" | "location.currentPosition" => continue,
                "location.servicesEnabled" => "true",
                "location.permission" => permission.as_str(),
                "notifications.setup" => r#"{"unapplied":[]}"#,
                _ => "null",
            };
            let Some(runtime) = runtime.upgrade() else {
                return;
            };
            runtime.reply(
                id,
                tokamak::PluginReply {
                    payload: payload.into(),
                    failed: false,
                    done: true,
                },
            );
        }
    });
}

/// The runtime of a Worker that records how its WebSocket closed, and the
/// request failures the runtime reports.
fn start_close_runtime(temporary: &Path) -> TestResult<(Runtime, PathBuf, Receiver<String>)> {
    let environment = WorkerEnvironment {
        services: BTreeMap::new(),
        vars: BTreeMap::new(),
        storage: vec![StorageBinding::Kv {
            name: "CLOSES".to_owned(),
            id: "closes".to_owned(),
        }],
    };
    start_reporting_runtime(temporary, close_worker_source(), &environment)
}

/// The runtime of `worker`, with `environment`, and the request failures it reports.
fn start_reporting_runtime(
    temporary: &Path,
    worker: &[u8],
    environment: &WorkerEnvironment,
) -> TestResult<(Runtime, PathBuf, Receiver<String>)> {
    let app = package(temporary, worker, environment)?;
    let (failed, failures) = mpsc::channel();
    let (runtime, state) = start_runtime(temporary, app, None, move |event| {
        if let Event::RequestFailed { message } = event {
            let _ = failed.send(message);
        }
    })?;
    Ok((runtime, state, failures))
}

/// What the Worker recorded as `name`, once it records it.
fn recorded(runtime: &Runtime, name: &str) -> TestResult<serde_json::Value> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let record = emit(runtime, name, "{}", TIMEOUT)?;
        if record != serde_json::Value::Null || Instant::now() >= deadline {
            return Ok(record);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Asserts the runtime reports no failure to `failures`, including one reported shortly after.
fn assert_no_failures(failures: &Receiver<String>) {
    let failure = failures.recv_timeout(Duration::from_millis(200));
    assert!(failure.is_err(), "reported {failure:?}");
}

fn d1_environment(migrations: &[&str]) -> WorkerEnvironment {
    WorkerEnvironment {
        services: BTreeMap::new(),
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
        services: BTreeMap::new(),
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
    uploadMetadata: await files.createMultipartUpload("upload", { customMetadata: { a: "\u0100".repeat(4096) } }).catch(error => error.message),
  };
}

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name, payload) {
    const input = JSON.parse(new TextDecoder().decode(payload));
    const reply = await this.reply(name, input, this.env.FILES);
    return { reply: new TextEncoder().encode(JSON.stringify(reply) ?? ""), listened: ["put", "get", "cloudflare"] };
  }

  async reply(name, input, files) {
    if (name === "cloudflare") return cloudflare(files);
    if (name === "put") return (await files.put(input.key, input.value)).etag;
    if (name !== "get") return undefined;
    const object = await files.get(input.key);
    return object === null ? "missing" : new Response(object.body).text();
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
    if (name !== "count") return { reply: new Uint8Array(), listened };
    const count = await db.prepare("SELECT count(*) AS count FROM hits").first("count");
    return { reply: new TextEncoder().encode(JSON.stringify(count)), listened };
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

/// A Worker whose WebSocket closes when it receives `close`, and which records
/// the close event its socket receives and the state it is in for any later
/// message.
fn close_worker_source() -> &'static [u8] {
    br#"
import { WorkerEntrypoint } from "cloudflare:workers";

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name) {
    return { reply: new TextEncoder().encode(await this.env.CLOSES.get(name) ?? ""), listened: ["close", "late"] };
  }
}

export default {
  async fetch(request, env, ctx) {
    const [client, server] = Object.values(new WebSocketPair());
    server.accept();
    server.addEventListener("message", ({ data }) => {
      if (data === "close") return server.close(4000, "bye");
      let sent = true;
      try { server.send("too late"); } catch { sent = false; }
      ctx.waitUntil(env.CLOSES.put("late", JSON.stringify([data, server.readyState, sent])));
    });
    server.addEventListener("close", ({ code, reason, wasClean }) => {
      ctx.waitUntil(env.CLOSES.put("close", JSON.stringify([code, reason, wasClean])));
    });
    return new Response(null, { status: 101, webSocket: client });
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
    connect_tls(state, HOST, open_tunnel(runtime)?)
}

/// A connection the gateway's CONNECT tunnel has opened, before TLS.
fn open_tunnel(runtime: &Runtime) -> TestResult<TcpStream> {
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy.set_read_timeout(Some(Duration::from_secs(2)))?;
    proxy
        .write_all(format!("CONNECT {HOST}:443 HTTP/1.1\r\nHost: {HOST}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;
    let proxy_response =
        read_header_block(&mut proxy).map_err(|error| format!("proxy response: {error}"))?;
    assert!(String::from_utf8(proxy_response)?.starts_with("HTTP/1.1 200"));
    Ok(proxy)
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

#[test]
fn sends_the_response_after_the_client_half_closes_its_sending_side() -> TestResult {
    let directory = tempfile::tempdir()?;
    let (runtime, state) = start_packaged_runtime(
        directory.path(),
        br"
export default {
  async fetch() {
    await new Promise(resolve => setTimeout(resolve, 200));
    return new Response(null, { status: 204 });
  }
};",
        &WorkerEnvironment::default(),
    )?;
    for tls_close in [false, true] {
        let mut tls = connect_gateway(&runtime, &state)?;
        tls.write_all(
            format!("GET / HTTP/1.1\r\nHost: {HOST}\r\nConnection: close\r\n\r\n").as_bytes(),
        )?;
        tls.flush()?;
        if tls_close {
            tls.conn.send_close_notify();
            tls.flush()?;
        } else {
            tls.sock.shutdown(Shutdown::Write)?;
        }
        let response = String::from_utf8(read_header_block(&mut tls)?)?;
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    }
    Ok(())
}
