//! An app reaches the development part through the entry point its binary
//! exports, as this test executable does.
#![cfg(feature = "native")]

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::io;

use base64::{Engine, engine::general_purpose::STANDARD};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tokamak::{DevProxyConfig, DevelopmentConfig, Event, PluginHandler, Runtime};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const HOST: &str = "dev.tokamak.local";

#[test]
fn marks_native_routes_for_the_authenticated_worker_relay() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Some((mut connection, headers)) = accept_before(&listener, deadline)? {
            if headers.starts_with("GET /_tokamak/app ") {
                connection.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nnative")?;
                return Ok(headers);
            }
            connection.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")?;
        }
        Err("the native request did not arrive".into())
    });
    let mut request = connect_gateway(&runtime, &temporary.path().join("state"))?;
    request.write_all(
        b"GET /_tokamak/app HTTP/1.1\r\nHost: dev.tokamak.local\r\nX-Tokamak-Session: forged\r\nX-Forwarded-Host: forged\r\n\r\n",
    )?;
    request.flush()?;
    assert!(read_http_response(&mut request)?.ends_with("native"));
    let headers = host_server.join().map_err(|_| "host server panicked")??;
    assert_eq!(header_value(&headers, "x-tokamak-dev-socket"), Some("1"));
    assert_eq!(
        header_value(&headers, "x-tokamak-session"),
        Some("test-session")
    );
    assert_eq!(header_value(&headers, "x-forwarded-host"), Some(HOST));
    assert_eq!(header_value(&headers, "x-forwarded-proto"), Some("https"));
    Ok(())
}

#[test]
fn forwards_http_and_websocket_traffic_to_the_host_server() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || serve_host(&listener));
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(
        "POST /api HTTP/1.1\r\nHost: dev.tokamak.local\r\nX-Repeated: café\r\nX-Repeated: 東京\r\nX-Tokamak-Dev-Socket: 1\r\nContent-Length: 3\r\n\r\n".as_bytes(),
    )?;
    http.write_all(&[0, 0xff, 1])?;
    http.flush()?;
    let response = read_http_response(&mut http)?;
    assert!(response.starts_with("HTTP/1.1 201 Created Here\r\n"));
    assert!(response.contains("set-cookie: first=1\r\n"));
    assert!(response.contains("set-cookie: second=2\r\n"));
    assert!(response.contains("x-text: 東京\r\n"));
    assert!(response.ends_with("host response"));

    let mut websocket = connect_gateway(&runtime, &state)?;
    websocket.write_all(
        b"GET /@vite/client HTTP/1.1\r\nHost: dev.tokamak.local\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Extensions: permessage-deflate; client_max_window_bits\r\n\r\n",
    )?;
    websocket.flush()?;
    let upgrade = read_header_block(&mut websocket)?;
    assert!(String::from_utf8(upgrade)?.starts_with("HTTP/1.1 101"));
    let (opcode, payload) = read_frame(&mut websocket)?;
    assert_eq!(opcode, 0x1);
    assert_eq!(payload, b"update");
    write_masked_frame(&mut websocket, 0x1, b"ping")?;
    let (opcode, payload) = read_frame(&mut websocket)?;
    assert_eq!(opcode, 0x1);
    assert_eq!(payload, b"pong");
    write_masked_frame(&mut websocket, 0x8, &[3, 232])?;

    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn relays_the_page_answer_to_a_host_close() -> TestResult {
    let (failed, failures) = mpsc::channel();
    let (listener, runtime, temporary) = start_test_runtime_reporting(move |event| {
        if let Event::RequestFailed { message } = event {
            let _ = failed.send(message);
        }
    })?;
    let host_server = thread::spawn(move || -> TestResult<(u8, Vec<u8>)> {
        let (mut websocket, _) = accept_closing_websocket(&listener)?;
        read_frame(&mut websocket)
    });
    let state = temporary.path().join("state");

    let mut websocket = open_page_websocket(&runtime, &state)?;
    // 4000, "bye"
    let close = [15, 160, b'b', b'y', b'e'];
    assert_eq!(read_frame(&mut websocket)?, (0x8, close.to_vec()));
    write_masked_frame(&mut websocket, 0x8, &close)?;

    let answer = host_server.join().map_err(|_| "host server panicked")??;
    assert_eq!(answer, (0x8, close.to_vec()));
    let failure = failures.recv_timeout(Duration::from_millis(200));
    assert!(failure.is_err(), "reported {failure:?}");
    Ok(())
}

#[test]
fn ends_quietly_when_the_host_closes_and_leaves() -> TestResult {
    let (failed, failures) = mpsc::channel();
    let (listener, runtime, temporary) = start_test_runtime_reporting(move |event| {
        if let Event::RequestFailed { message } = event {
            let _ = failed.send(message);
        }
    })?;
    let host_server = thread::spawn(move || -> TestResult {
        let (websocket, _) = accept_closing_websocket(&listener)?;
        drop(websocket);
        Ok(())
    });
    let state = temporary.path().join("state");

    let mut websocket = open_page_websocket(&runtime, &state)?;
    assert_eq!(read_frame(&mut websocket)?.0, 0x8);
    host_server.join().map_err(|_| "host server panicked")??;

    let failure = failures.recv_timeout(Duration::from_millis(200));
    assert!(failure.is_err(), "reported {failure:?}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn closes_client_websocket_after_an_upstream_reset() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || serve_resetting_websocket(&listener));
    let state = temporary.path().join("state");

    let mut websocket = connect_gateway(&runtime, &state)?;
    websocket.write_all(
        b"GET /@vite/client HTTP/1.1\r\nHost: dev.tokamak.local\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: vite-hmr\r\n\r\n",
    )?;
    websocket.flush()?;
    let upgrade = String::from_utf8(read_header_block(&mut websocket)?)?;
    if !upgrade.starts_with("HTTP/1.1 101") {
        return Err(format!("unexpected WebSocket response: {upgrade:?}").into());
    }
    let (opcode, payload) = read_frame(&mut websocket)?;
    assert_eq!(opcode, 0x8);
    assert_eq!(&payload[..2], &[3, 243]);
    assert_eq!(&payload[2..], b"development server connection failed");

    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn retries_get_after_the_host_server_restarts() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || serve_restarting_host(&listener));
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(b"GET / HTTP/1.1\r\nHost: dev.tokamak.local\r\n\r\n")?;
    http.flush()?;
    let response = read_http_response(&mut http)?;

    host_server.join().map_err(|_| "host server panicked")??;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.ends_with("ok"));
    Ok(())
}

#[test]
fn does_not_retry_post_after_an_upstream_disconnect() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || serve_interrupted_post(&listener));
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(b"POST /api HTTP/1.1\r\nHost: dev.tokamak.local\r\nContent-Length: 0\r\n\r\n")?;
    http.flush()?;
    let response = read_http_response(&mut http)?;

    host_server.join().map_err(|_| "host server panicked")??;
    assert!(response.starts_with("HTTP/1.1 500"));
    Ok(())
}

#[test]
fn preserves_opaque_event_payloads_and_replies() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let bytes = [0, 255, 128, b'{', 0];
    let host_server = thread::spawn(move || -> TestResult {
        let (mut socket, _) = accept_dev_socket(&listener)?;
        answer_event(&mut socket)?;
        let (opcode, frame) = read_frame(&mut socket)?;
        assert_eq!(opcode, 0x2);
        let length = u32::from_le_bytes(frame[4..8].try_into()?) as usize;
        let header: Value = serde_json::from_slice(&frame[8..8 + length])?;
        assert_eq!(header["name"], "example.binary");
        assert_eq!(&frame[8 + length..], &bytes);
        let reply = encode_payload(&json!({ "type": "reply", "id": header["id"] }), &bytes)?;
        write_frame(&mut socket, 0x2, &reply, false)?;
        Ok(())
    });
    assert_eq!(
        runtime.emit("example.binary", &bytes, Duration::from_secs(5))?,
        bytes
    );
    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn delivers_events_through_the_dev_socket() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult<(String, Vec<String>)> {
        let (mut socket, headers) = accept_dev_socket(&listener)?;
        let events = [answer_event(&mut socket)?, answer_event(&mut socket)?];
        Ok((headers, events.to_vec()))
    });

    let reply = runtime.emit("example.ping", br#"{"id":"1"}"#, Duration::from_secs(5))?;

    let (headers, events) = host_server.join().map_err(|_| "host server panicked")??;
    assert_eq!(reply, br#"{"echo":"example.ping"}"#);
    assert_eq!(
        header_value(&headers, "x-tokamak-session"),
        Some("test-session")
    );
    assert_eq!(
        events,
        [
            r#"{"type":"event","id":0,"name":"start","event":{"foreground":true}}"#,
            r#"{"type":"event","id":1,"name":"example.ping","event":{"id":"1"}}"#,
        ]
    );
    Ok(())
}

/// Shell plugins that report each request as text.
struct Shell(mpsc::Sender<(u64, String)>);

impl PluginHandler for Shell {
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let arguments = String::from_utf8_lossy(arguments);
        let _ = self
            .0
            .send((id, format!("call {plugin}.{method} {arguments}")));
    }

    fn call_with_body(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let arguments = String::from_utf8_lossy(arguments);
        let _ = self
            .0
            .send((id, format!("body {plugin}.{method} {arguments}")));
    }

    fn chunk(&self, id: u64, chunk: &tokamak::PluginReply) {
        let payload = String::from_utf8_lossy(&chunk.payload);
        let _ = self.0.send((id, format!("chunk {payload} {}", chunk.done)));
    }

    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let arguments = String::from_utf8_lossy(arguments);
        let _ = self
            .0
            .send((id, format!("subscribe {plugin}.{method} {arguments}")));
    }

    fn cancel(&self, id: u64) {
        let _ = self.0.send((id, "cancel".to_owned()));
    }
}

#[test]
fn runs_the_development_workers_plugin_calls_on_the_device() -> TestResult {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let temporary = tempfile::tempdir()?;
    let (sender, requests) = mpsc::channel();
    let runtime = Runtime::start_development(
        DevelopmentConfig {
            state_dir: temporary.path().join("state"),
            host: HOST.to_owned(),
            proxy: DevProxyConfig {
                endpoint: format!("http://127.0.0.1:{}", listener.local_addr()?.port()),
                session_token: "test-session".to_owned(),
            },
            foreground: false,
            plugins: Some(Arc::new(Shell(sender))),
        },
        |_| {},
    )?;
    let (mut socket, _) = accept_dev_socket(&listener)?;
    answer_event(&mut socket)?;
    let next_request = || requests.recv_timeout(Duration::from_secs(5));

    send_message(
        &mut socket,
        &json!({ "type": "call", "id": 1, "plugin": "tokamak", "method": "lifecycleStage", "arguments": null }),
    )?;
    send_message(
        &mut socket,
        &json!({ "type": "call", "id": 2, "plugin": "location", "method": "getCurrentPosition", "arguments": { "timeout": 1 } }),
    )?;
    let (call, request) = next_request()?;
    assert_eq!(request, r#"call location.getCurrentPosition {"timeout":1}"#);
    runtime.reply(call, value(br#"{"latitude":51.5}"#, true));
    send_message(
        &mut socket,
        &json!({ "type": "subscribe", "id": 3, "plugin": "location", "method": "watchPosition", "arguments": null }),
    )?;
    let (subscription, request) = next_request()?;
    assert_eq!(request, "subscribe location.watchPosition null");
    runtime.reply(subscription, value(b"1", false));
    let mut results = [
        read_message(&mut socket)?,
        read_message(&mut socket)?,
        read_message(&mut socket)?,
    ];
    results.sort_by_key(|result| result["id"].as_u64());
    send_message(&mut socket, &json!({ "type": "cancel", "id": 3 }))?;

    assert_eq!(
        results,
        [
            json!({ "type": "result", "id": 1, "result": { "value": "background", "done": true } }),
            json!({ "type": "result", "id": 2, "result": { "value": { "latitude": 51.5 }, "done": true } }),
            json!({ "type": "result", "id": 3, "result": { "value": 1, "done": false } }),
        ]
    );
    assert_eq!(next_request()?, (subscription, "cancel".to_owned()));
    for (id, disconnect) in [(4, false), (5, true)] {
        send_message(
            &mut socket,
            &json!({ "type": "call", "id": id, "plugin": "location", "method": "getCurrentPosition" }),
        )?;
        let (call, request) = next_request()?;
        assert_eq!(request, "call location.getCurrentPosition null");
        if disconnect {
            socket.shutdown(Shutdown::Both)?;
        } else {
            send_message(&mut socket, &json!({ "type": "cancel", "id": id }))?;
        }
        assert_eq!(next_request()?, (call, "cancel".to_owned()));
        runtime.reply(call, value(br#""late""#, true));
    }
    Ok(())
}

#[test]
fn relays_a_call_body_as_the_device_pulls_each_chunk() -> TestResult {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let temporary = tempfile::tempdir()?;
    let (sender, requests) = mpsc::channel();
    let runtime = Runtime::start_development(
        DevelopmentConfig {
            state_dir: temporary.path().join("state"),
            host: HOST.to_owned(),
            proxy: DevProxyConfig {
                endpoint: format!("http://127.0.0.1:{}", listener.local_addr()?.port()),
                session_token: "test-session".to_owned(),
            },
            foreground: false,
            plugins: Some(Arc::new(Shell(sender))),
        },
        |_| {},
    )?;
    let (mut socket, _) = accept_dev_socket(&listener)?;
    answer_event(&mut socket)?;
    let next_request = || requests.recv_timeout(Duration::from_secs(5));
    send_message(
        &mut socket,
        &json!({ "type": "callWithBody", "id": 7, "plugin": "files", "method": "write", "arguments": {} }),
    )?;
    let (call, request) = next_request()?;
    assert_eq!(request, "body files.write {}");

    for (payload, done) in [(&b"part"[..], false), (&b""[..], true)] {
        runtime.pull(call);
        assert_eq!(
            read_message(&mut socket)?,
            json!({ "type": "pull", "id": 7 })
        );
        let chunk = json!({ "type": "chunk", "id": 7, "done": done });
        write_frame(&mut socket, 0x2, &encode_payload(&chunk, payload)?, false)?;
        let text = String::from_utf8_lossy(payload);
        assert_eq!(next_request()?, (call, format!("chunk {text} {done}")));
    }
    runtime.reply(call, value(br#""written""#, true));

    assert_eq!(
        read_message(&mut socket)?,
        json!({ "type": "result", "id": 7, "result": { "value": "written", "done": true } })
    );
    Ok(())
}

#[test]
fn reopens_the_dev_socket_after_it_closes() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let (reopened, reopening) = mpsc::channel();
    let host_server = thread::spawn(move || -> TestResult<String> {
        let (mut first, _) = accept_dev_socket(&listener)?;
        answer_event(&mut first)?;
        drop(first);
        let (mut second, _) = accept_dev_socket(&listener)?;
        reopened.send(())?;
        answer_event(&mut second)
    });
    reopening.recv_timeout(Duration::from_secs(10))?;

    let reply = runtime.emit("suspend", b"{}", Duration::from_secs(5));

    assert_eq!(
        host_server.join().map_err(|_| "host server panicked")??,
        r#"{"type":"event","id":1,"name":"suspend","event":{}}"#
    );
    assert_eq!(reply?, b"null");
    Ok(())
}

#[test]
fn assembles_a_reply_sent_in_fragments() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult {
        let (mut socket, _) = accept_dev_socket(&listener)?;
        answer_event(&mut socket)?;
        let event = read_message(&mut socket)?;
        let reply =
            encode_message(&json!({ "type": "reply", "id": event["id"], "reply": "joined" }))?;
        let (first, last) = reply.split_at(reply.len() / 2);
        socket.write_all(&[0x02, u8::try_from(first.len())?])?;
        socket.write_all(first)?;
        write_frame(&mut socket, 0x0, last, false)
    });

    let reply = runtime.emit("example.ping", b"{}", Duration::from_secs(5))?;

    host_server.join().map_err(|_| "host server panicked")??;
    assert_eq!(reply, br#""joined""#);
    Ok(())
}

#[test]
fn fails_an_event_the_plugin_answers_with_an_error_or_too_late() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult {
        let (mut socket, _) = accept_dev_socket(&listener)?;
        answer_event(&mut socket)?;
        for answer in [
            serde_json::json!({ "type": "error", "message": "listener exploded" }),
            serde_json::json!({ "type": "reply", "reply": "late" }),
        ] {
            let event = read_message(&mut socket)?;
            let mut answer = answer;
            answer["id"] = event["id"].clone();
            if answer["type"] == "reply" {
                thread::sleep(Duration::from_millis(500));
            }
            send_message(&mut socket, &answer)?;
        }
        Ok(())
    });

    let failed = runtime.emit("example.ping", b"{}", Duration::from_secs(5));
    let late = runtime.emit("example.ping", b"{}", Duration::from_millis(200));

    host_server.join().map_err(|_| "host server panicked")??;
    assert_eq!(
        failed.err().ok_or("the event completed")?.to_string(),
        "listener exploded"
    );
    assert_eq!(
        late.err().ok_or("the event completed")?.to_string(),
        "example.ping did not complete before its deadline"
    );
    Ok(())
}

#[test]
fn fails_an_event_no_dev_socket_answers_before_its_deadline() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    drop(listener);

    let Err(error) = runtime.emit("example.ping", b"{}", Duration::from_millis(200)) else {
        return Err("the event was delivered".into());
    };

    assert_eq!(
        error.to_string(),
        "example.ping timed out waiting for start"
    );
    Ok(())
}

#[test]
fn fetches_a_path_from_the_host_server() -> TestResult {
    let (listener, runtime, _temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult {
        let mut host = accept_request(&listener, "GET /sounds/chime.caf ")?;
        host.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nchime")?;
        Ok(())
    });

    assert_eq!(
        runtime.fetch("/sounds/chime.caf", Duration::from_secs(5))?,
        b"chime"
    );
    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn forwards_chunked_host_responses_as_they_arrive() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let host_server = thread::spawn(move || -> TestResult {
        let (mut host, _) = accept_page(&listener)?;
        host.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\npart\r\n",
        )?;
        host.flush()?;
        release_receiver.recv_timeout(Duration::from_secs(2))?;
        host.write_all(b"5\r\n two!\r\n0\r\n\r\n")?;
        Ok(())
    });
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(b"GET /stream HTTP/1.1\r\nHost: dev.tokamak.local\r\n\r\n")?;
    http.flush()?;
    let headers = String::from_utf8(read_header_block(&mut http)?)?;
    assert!(headers.contains("transfer-encoding: chunked\r\n"));
    assert_eq!(read_chunk(&mut http)?, b"part");
    release_sender.send(())?;
    assert_eq!(read_chunk(&mut http)?, b" two!");
    assert!(read_chunk(&mut http)?.is_empty());

    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn forwards_close_delimited_host_responses_as_they_arrive() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let host_server = thread::spawn(move || -> TestResult {
        let (mut host, _) = accept_page(&listener)?;
        host.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\npart")?;
        host.flush()?;
        release_receiver.recv_timeout(Duration::from_secs(2))?;
        host.write_all(b" two!")?;
        Ok(())
    });
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(b"GET /stream HTTP/1.1\r\nHost: dev.tokamak.local\r\n\r\n")?;
    http.flush()?;
    let headers = String::from_utf8(read_header_block(&mut http)?)?;
    assert!(headers.contains("transfer-encoding: chunked\r\n"));
    assert!(!headers.contains("content-length:"));
    assert_eq!(read_chunk(&mut http)?, b"part");
    release_sender.send(())?;
    assert_eq!(read_chunk(&mut http)?, b" two!");
    assert!(read_chunk(&mut http)?.is_empty());

    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

#[test]
fn stops_an_idle_stream_when_the_runtime_stops() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult {
        let (mut host, _) = accept_page(&listener)?;
        host.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        )?;
        host.flush()?;
        host.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut byte = [0; 1];
        match host.read(&mut byte) {
            Ok(0) => Ok(()),
            Ok(_) => Err("idle host stream received unexpected data".into()),
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                Err("idle host stream was not cancelled".into())
            }
            Err(error) => Err(error.into()),
        }
    });
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(b"GET /idle HTTP/1.1\r\nHost: dev.tokamak.local\r\n\r\n")?;
    http.flush()?;
    let headers = String::from_utf8(read_header_block(&mut http)?)?;
    assert!(headers.contains("transfer-encoding: chunked\r\n"));
    drop(http);
    drop(runtime);

    host_server.join().map_err(|_| "host server panicked")??;
    Ok(())
}

fn start_test_runtime() -> TestResult<(TcpListener, Runtime, tempfile::TempDir)> {
    start_test_runtime_reporting(|_| {})
}

fn start_test_runtime_reporting(
    events: impl Fn(Event) + Send + Sync + 'static,
) -> TestResult<(TcpListener, Runtime, tempfile::TempDir)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let endpoint = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let temporary = tempfile::tempdir()?;
    let runtime = Runtime::start_development(
        DevelopmentConfig {
            state_dir: temporary.path().join("state"),
            host: HOST.to_owned(),
            proxy: DevProxyConfig {
                endpoint,
                session_token: "test-session".to_owned(),
            },
            foreground: true,
            plugins: None,
        },
        events,
    )?;
    Ok((listener, runtime, temporary))
}

fn serve_host(listener: &TcpListener) -> TestResult {
    let (mut http, text) = accept_page(listener)?;
    let (request_line, headers) = text.split_once("\r\n").ok_or("missing request line")?;
    assert_eq!(request_line, "POST /api HTTP/1.1");
    let mut headers: Vec<_> = headers.lines().filter(|line| !line.is_empty()).collect();
    headers.sort_unstable();
    assert_eq!(
        headers,
        [
            "content-length: 3",
            "host: dev.tokamak.local",
            "x-forwarded-host: dev.tokamak.local",
            "x-forwarded-proto: https",
            "x-repeated: café",
            "x-repeated: 東京",
            "x-tokamak-session: test-session",
        ]
    );
    let mut body = [0; 3];
    http.read_exact(&mut body)?;
    assert_eq!(body, [0, 0xff, 1]);
    http.write_all(
        "HTTP/1.1 201 Created Here\r\nSet-Cookie: first=1\r\nSet-Cookie: second=2\r\nX-Text: 東京\r\nContent-Length: 13\r\nConnection: close\r\n\r\nhost response".as_bytes(),
    )?;

    let (mut websocket, text) = accept_page(listener)?;
    let key = header_value(&text, "sec-websocket-key").ok_or("missing WebSocket key")?;
    assert_eq!(header_value(&text, "sec-websocket-extensions"), None);
    let accept = websocket_accept(key);
    write!(
        websocket,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    write_frame(&mut websocket, 0x1, b"update", false)?;
    let (opcode, payload) = read_frame(&mut websocket)?;
    assert_eq!(opcode, 0x1);
    assert_eq!(payload, b"ping");
    write_frame(&mut websocket, 0x1, b"pong", false)?;
    let (opcode, payload) = read_frame(&mut websocket)?;
    assert_eq!(opcode, 0x8);
    assert_eq!(payload, [3, 232]);
    Ok(())
}

fn serve_restarting_host(listener: &TcpListener) -> TestResult {
    let mut first = accept_request(listener, "GET / HTTP/1.1")?;
    first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\no")?;
    drop(first);

    for _ in 0..2 {
        drop(accept_request(listener, "GET / HTTP/1.1")?);
    }

    let mut retry = accept_request(listener, "GET / HTTP/1.1")?;
    retry.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")?;
    Ok(())
}

/// Accepts the page's WebSocket and sends a close frame of 4000, "bye".
fn accept_closing_websocket(listener: &TcpListener) -> TestResult<(TcpStream, String)> {
    let (mut websocket, headers) = accept_page(listener)?;
    let key = header_value(&headers, "sec-websocket-key").ok_or("missing WebSocket key")?;
    let accept = websocket_accept(key);
    write!(
        websocket,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    write_frame(&mut websocket, 0x8, &[15, 160, b'b', b'y', b'e'], false)?;
    Ok((websocket, headers))
}

/// The page's WebSocket to `/socket`, upgraded through the gateway.
fn open_page_websocket(
    runtime: &Runtime,
    state: &Path,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut websocket = connect_gateway(runtime, state)?;
    websocket.write_all(
        b"GET /socket HTTP/1.1\r\nHost: dev.tokamak.local\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
    )?;
    websocket.flush()?;
    let upgrade = read_header_block(&mut websocket)?;
    if !String::from_utf8(upgrade.clone())?.starts_with("HTTP/1.1 101") {
        return Err(format!(
            "unexpected WebSocket response: {:?}",
            String::from_utf8_lossy(&upgrade)
        )
        .into());
    }
    Ok(websocket)
}

#[cfg(unix)]
fn serve_resetting_websocket(listener: &TcpListener) -> TestResult {
    let (mut websocket, headers) = accept_page(listener)?;
    assert!(headers.starts_with("GET /@vite/client HTTP/1.1"));
    let key = header_value(&headers, "sec-websocket-key").ok_or("missing WebSocket key")?;
    let accept = websocket_accept(key);
    write!(
        websocket,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    websocket.flush()?;
    thread::sleep(Duration::from_millis(100));
    reset_connection(&websocket)?;
    Ok(())
}

#[cfg(unix)]
fn reset_connection(stream: &TcpStream) -> TestResult {
    use std::os::fd::AsRawFd;

    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    let linger_size = libc::socklen_t::try_from(std::mem::size_of_val(&linger))?;
    let result = unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&raw const linger).cast(),
            linger_size,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error().into())
    }
}

fn serve_interrupted_post(listener: &TcpListener) -> TestResult {
    let mut post = accept_request(listener, "POST /api HTTP/1.1")?;
    post.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\no")?;
    drop(post);

    if accept_page_before(listener, Duration::from_millis(300))?.is_some() {
        return Err("POST request was retried".into());
    }
    Ok(())
}

fn accept_request(listener: &TcpListener, expected: &str) -> TestResult<TcpStream> {
    let (stream, request) = accept_page(listener)?;
    assert!(request.starts_with(expected));
    Ok(stream)
}

/// The next connection to `listener` other than the dev socket, which it
/// refuses, with its request headers.
fn accept_page(listener: &TcpListener) -> TestResult<(TcpStream, String)> {
    Ok(accept_page_before(listener, Duration::from_secs(2))?
        .ok_or("host server did not receive a request")?)
}

fn accept_page_before(
    listener: &TcpListener,
    timeout: Duration,
) -> TestResult<Option<(TcpStream, String)>> {
    let deadline = Instant::now() + timeout;
    while let Some((mut stream, headers)) = accept_before(listener, deadline)? {
        if header_value(&headers, "x-tokamak-dev-socket").is_none() {
            return Ok(Some((stream, headers)));
        }
        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")?;
    }
    Ok(None)
}

/// The next dev socket connection to `listener`, accepted, with its request
/// headers.
fn accept_dev_socket(listener: &TcpListener) -> TestResult<(TcpStream, String)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some((mut stream, headers)) = accept_before(listener, deadline)? {
        if header_value(&headers, "x-tokamak-dev-socket") == Some("1") {
            let key = header_value(&headers, "sec-websocket-key").ok_or("missing WebSocket key")?;
            write!(
                stream,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                websocket_accept(key)
            )?;
            return Ok((stream, headers));
        }
    }
    Err("the dev socket did not connect".into())
}

/// Reads an event from the dev socket and replies to it with its name,
/// returning the event's message.
fn answer_event(socket: &mut TcpStream) -> TestResult<String> {
    let message = read_message(socket)?;
    send_message(
        socket,
        &json!({
            "type": "reply", "id": message["id"],
            "reply": if message["name"] == "example.ping" { json!({ "echo": message["name"] }) } else { Value::Null },
        }),
    )?;
    // Keep event assertions independent of routing metadata defaults.
    Ok(format!(
        r#"{{"type":"event","id":{},"name":{},"event":{}}}"#,
        message["id"], message["name"], message["event"]
    ))
}

fn encode_message(message: &Value) -> TestResult<Vec<u8>> {
    let mut header = message.clone();
    let payload = match message["type"].as_str() {
        Some("call" | "callWithBody" | "subscribe") => header
            .as_object_mut()
            .ok_or("no header")?
            .remove("arguments")
            .unwrap_or(Value::Null)
            .to_string()
            .into_bytes(),
        Some("reply") => header
            .as_object_mut()
            .ok_or("no header")?
            .remove("reply")
            .unwrap_or(Value::Null)
            .to_string()
            .into_bytes(),
        Some("error") => header
            .as_object_mut()
            .ok_or("no header")?
            .remove("message")
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
            .into_bytes(),
        _ => vec![],
    };
    encode_payload(&header, &payload)
}

fn encode_payload(header: &Value, payload: &[u8]) -> TestResult<Vec<u8>> {
    let metadata = serde_json::to_vec(header)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&u32::try_from(4 + metadata.len() + payload.len())?.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(metadata.len())?.to_le_bytes());
    bytes.extend_from_slice(&metadata);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

fn send_message(socket: &mut TcpStream, message: &Value) -> TestResult {
    write_frame(socket, 0x2, &encode_message(message)?, false)
}

fn read_message(socket: &mut TcpStream) -> TestResult<Value> {
    let (opcode, bytes) = read_frame(socket)?;
    assert_eq!(opcode, 0x2);
    let metadata = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let mut message: Value = serde_json::from_slice(&bytes[8..8 + metadata])?;
    let payload = &bytes[8 + metadata..];
    let payload: Value = if payload.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(payload)?
    };
    if message["type"] == "event" {
        message["event"] = payload;
    } else if message["type"] == "result" {
        let key = if message["failed"] == true {
            "error"
        } else {
            "value"
        };
        message["result"] = json!({ key: payload, "done": message["done"] });
    }
    let header = message.as_object_mut().ok_or("no header")?;
    header.remove("failed");
    header.remove("done");
    Ok(message)
}

/// The next connection to `listener` before `deadline`, with its request
/// headers.
fn accept_before(
    listener: &TcpListener,
    deadline: Instant,
) -> TestResult<Option<(TcpStream, String)>> {
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                let headers = String::from_utf8(read_header_block(&mut stream)?)?;
                return Ok(Some((stream, headers)));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
}

fn connect_gateway(
    runtime: &Runtime,
    state: &Path,
) -> TestResult<StreamOwned<ClientConnection, TcpStream>> {
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy
        .write_all(format!("CONNECT {HOST}:443 HTTP/1.1\r\nHost: {HOST}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;
    let response = read_header_block(&mut proxy)?;
    if !String::from_utf8(response)?.starts_with("HTTP/1.1 200") {
        return Err("gateway CONNECT failed".into());
    }
    connect_tls(state, HOST, proxy)
}

fn read_http_response(stream: &mut StreamOwned<ClientConnection, TcpStream>) -> TestResult<String> {
    let headers = read_header_block(stream)?;
    let text = String::from_utf8(headers)?;
    let length = header_value(&text, "content-length")
        .ok_or("missing response content length")?
        .parse::<usize>()?;
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(format!("{text}{}", String::from_utf8(body)?))
}

fn read_chunk(stream: &mut impl Read) -> TestResult<Vec<u8>> {
    let line = String::from_utf8(read_line(stream)?)?;
    let size = usize::from_str_radix(line.trim(), 16)?;
    let mut body = vec![0; size];
    stream.read_exact(&mut body)?;
    let mut terminator = [0; 2];
    stream.read_exact(&mut terminator)?;
    assert_eq!(terminator, *b"\r\n");
    Ok(body)
}

fn read_line(stream: &mut impl Read) -> TestResult<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return Ok(line);
        }
    }
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

fn header_value<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}

fn websocket_accept(key: &str) -> String {
    STANDARD.encode(Sha1::digest(
        format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ))
}

fn write_masked_frame(stream: &mut impl Write, opcode: u8, payload: &[u8]) -> TestResult {
    write_frame(stream, opcode, payload, true)
}

fn write_frame(stream: &mut impl Write, opcode: u8, payload: &[u8], mask: bool) -> TestResult {
    let key = [1, 2, 3, 4];
    let mask_bit = if mask { 0x80 } else { 0 };
    let length = u8::try_from(payload.len())?;
    stream.write_all(&[0x80 | opcode, mask_bit | length])?;
    if mask {
        stream.write_all(&key)?;
        let payload: Vec<_> = payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ key[index % key.len()])
            .collect();
        stream.write_all(&payload)?;
    } else {
        stream.write_all(payload)?;
    }
    stream.flush()?;
    Ok(())
}

fn read_frame(stream: &mut impl Read) -> TestResult<(u8, Vec<u8>)> {
    let mut header = [0; 2];
    stream.read_exact(&mut header)?;
    let length = usize::from(header[1] & 0x7f);
    let mut mask = [0; 4];
    if header[1] & 0x80 != 0 {
        stream.read_exact(&mut mask)?;
    }
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload)?;
    if header[1] & 0x80 != 0 {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % mask.len()];
        }
    }
    Ok((header[0] & 0x0f, payload))
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

fn value(payload: &[u8], done: bool) -> tokamak::PluginReply {
    tokamak::PluginReply {
        payload: payload.to_vec(),
        failed: false,
        done,
    }
}
