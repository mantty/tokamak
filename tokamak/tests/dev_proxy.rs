#![cfg(feature = "native")]

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::io;

use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::sha::sha1;
use openssl::ssl::{SslConnector, SslFiletype, SslMethod, SslStream};
use tokamak::{DevProxyConfig, DevelopmentConfig, Runtime};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const HOST: &str = "dev.tokamak.local";

#[test]
fn forwards_http_and_websocket_traffic_to_the_host_server() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || serve_host(&listener));
    let state = temporary.path().join("state");

    let mut http = connect_gateway(&runtime, &state)?;
    http.write_all(
        "POST /api HTTP/1.1\r\nHost: dev.tokamak.local\r\nX-Repeated: café\r\nX-Repeated: 東京\r\nContent-Length: 3\r\n\r\n".as_bytes(),
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
fn forwards_chunked_host_responses_as_they_arrive() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let (release_sender, release_receiver) = mpsc::sync_channel(1);
    let host_server = thread::spawn(move || -> TestResult {
        let (mut host, _) = listener.accept()?;
        read_header_block(&mut host)?;
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
fn stops_an_idle_stream_when_the_runtime_stops() -> TestResult {
    let (listener, runtime, temporary) = start_test_runtime()?;
    let host_server = thread::spawn(move || -> TestResult {
        let (mut host, _) = listener.accept()?;
        read_header_block(&mut host)?;
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
        },
        |_| {},
    )?;
    Ok((listener, runtime, temporary))
}

fn serve_host(listener: &TcpListener) -> TestResult {
    let (mut http, _) = listener.accept()?;
    http.set_read_timeout(Some(Duration::from_secs(2)))?;
    let headers = read_header_block(&mut http)?;
    let text = String::from_utf8(headers)?;
    assert!(text.starts_with("POST /api HTTP/1.1"));
    assert!(text.contains("Host: dev.tokamak.local\r\n"));
    assert!(text.contains("X-Tokamak-Session: test-session\r\n"));
    assert!(text.contains("x-repeated: café\r\n"));
    assert!(text.contains("x-repeated: 東京\r\n"));
    let mut body = [0; 3];
    http.read_exact(&mut body)?;
    assert_eq!(body, [0, 0xff, 1]);
    http.write_all(
        "HTTP/1.1 201 Created Here\r\nSet-Cookie: first=1\r\nSet-Cookie: second=2\r\nX-Text: 東京\r\nContent-Length: 13\r\nConnection: close\r\n\r\nhost response".as_bytes(),
    )?;

    let (mut websocket, _) = listener.accept()?;
    websocket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let headers = read_header_block(&mut websocket)?;
    let text = String::from_utf8(headers)?;
    let key = header_value(&text, "sec-websocket-key").ok_or("missing WebSocket key")?;
    assert!(!text.contains("Sec-WebSocket-Extensions:"));
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
    listener.set_nonblocking(true)?;
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

#[cfg(unix)]
fn serve_resetting_websocket(listener: &TcpListener) -> TestResult {
    let (mut websocket, _) = listener.accept()?;
    websocket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let headers = String::from_utf8(read_header_block(&mut websocket)?)?;
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
    listener.set_nonblocking(true)?;
    let mut post = accept_request(listener, "POST /api HTTP/1.1")?;
    post.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\no")?;
    drop(post);

    if accept_before(listener, Duration::from_millis(300))?.is_some() {
        return Err("POST request was retried".into());
    }
    Ok(())
}

fn accept_request(listener: &TcpListener, expected: &str) -> TestResult<TcpStream> {
    let mut stream = accept_before(listener, Duration::from_secs(2))?
        .ok_or("host server did not receive a request")?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let request = String::from_utf8(read_header_block(&mut stream)?)?;
    assert!(request.starts_with(expected));
    Ok(stream)
}

fn accept_before(listener: &TcpListener, timeout: Duration) -> TestResult<Option<TcpStream>> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(Some(stream)),
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
}

fn connect_gateway(runtime: &Runtime, state: &Path) -> TestResult<SslStream<TcpStream>> {
    let mut proxy = TcpStream::connect(("127.0.0.1", runtime.port()))?;
    proxy
        .write_all(format!("CONNECT {HOST}:443 HTTP/1.1\r\nHost: {HOST}:443\r\n\r\n").as_bytes())?;
    proxy.flush()?;
    let response = read_header_block(&mut proxy)?;
    if !String::from_utf8(response)?.starts_with("HTTP/1.1 200") {
        return Err("gateway CONNECT failed".into());
    }
    let mut connector = SslConnector::builder(SslMethod::tls())?;
    connector.set_ca_file(state.join("ca.cert.pem"))?;
    connector.set_certificate_file(state.join("client.cert.pem"), SslFiletype::PEM)?;
    connector.set_private_key_file(state.join("client.key.pem"), SslFiletype::PEM)?;
    Ok(connector.build().connect(HOST, proxy)?)
}

fn read_http_response(stream: &mut SslStream<TcpStream>) -> TestResult<String> {
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
    STANDARD.encode(sha1(
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
