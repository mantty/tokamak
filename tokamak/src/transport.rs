use flume::{Receiver, RecvTimeoutError, Sender, TryRecvError, bounded};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use hyper::{
    StatusCode,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use rustls::{AlertDescription, ServerConfig, ServerConnection, StreamOwned};
use serde::Serialize;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::gateway::{WebSocketBridge, WebSocketInbound, WebSocketOutbound, WebSocketOutgoing};
use crate::readiness::Readiness;

const MAX_HEADERS: usize = 64 * 1024;
const MAX_HTTP_BODY: usize = 250 * 1024 * 1024;
const MAX_WEBSOCKET_BODY: usize = 16 * 1024 * 1024;
const RESPONSE_STREAM_QUEUE: usize = 8;
const RESPONSE_STREAM_POLL: Duration = Duration::from_millis(100);
const WEBSOCKET_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) type BodyChunk = Result<Vec<u8>, String>;
pub(super) type TlsStream = StreamOwned<ServerConnection, TcpStream>;

/// Observes connection failures without consuming pipelined HTTP requests.
pub(super) fn client_disconnected(
    stream: &mut TlsStream,
    readiness: &mut Readiness,
) -> io::Result<bool> {
    if readiness.closed()? {
        return Ok(true);
    }
    let state = stream
        .conn
        .process_new_packets()
        .map_err(io::Error::other)?;
    if state.peer_has_closed() {
        return Ok(false);
    }
    if !stream.conn.wants_read() {
        return Ok(false);
    }
    match stream.conn.read_tls(&mut stream.sock) {
        Ok(0) => Ok(false),
        Ok(_) => stream
            .conn
            .process_new_packets()
            .map(|_| false)
            .map_err(io::Error::other),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

#[derive(Clone, Serialize)]
pub(super) struct HttpRequest {
    /// Whether the connection stays open after the response, decided by the gateway.
    #[serde(skip_serializing)]
    pub(super) persistent: bool,
    pub(super) method: String,
    pub(super) target: String,
    pub(super) url: String,
    #[serde(serialize_with = "crate::network::headers::serialize")]
    pub(super) headers: HeaderMap,
    #[serde(skip_serializing)]
    pub(super) body: Option<Vec<u8>>,
}

pub(super) struct HttpResponse {
    pub(super) status: u16,
    pub(super) status_text: String,
    pub(super) headers: HeaderMap,
    pub(super) body: HttpBody,
}

pub(super) enum HttpBody {
    Buffered(Vec<u8>),
    Stream(BodyStream),
}

pub(super) struct BodyStream {
    receiver: Receiver<BodyChunk>,
    cancelled: CancellationToken,
}

impl BodyStream {
    #[cfg(test)]
    pub(super) fn recv(&self) -> Result<BodyChunk, flume::RecvError> {
        self.receiver.recv()
    }
}

pub(super) struct WebSocketFrame {
    pub(super) final_frame: bool,
    pub(super) opcode: u8,
    pub(super) payload: Vec<u8>,
}

impl HttpResponse {
    pub(super) fn text(status: u16, body: &str) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        Self::buffered(status, headers, body.as_bytes().to_vec())
    }

    pub(super) fn buffered(status: u16, headers: HeaderMap, body: Vec<u8>) -> Self {
        Self {
            status,
            status_text: StatusCode::from_u16(status)
                .ok()
                .and_then(|code| code.canonical_reason())
                .unwrap_or("")
                .to_owned(),
            headers,
            body: HttpBody::Buffered(body),
        }
    }
}

pub(super) fn response_stream() -> (Sender<BodyChunk>, CancellationToken, HttpBody) {
    let (sender, receiver) = bounded(RESPONSE_STREAM_QUEUE);
    let cancelled = CancellationToken::new();
    (
        sender,
        cancelled.clone(),
        HttpBody::Stream(BodyStream {
            receiver,
            cancelled,
        }),
    )
}

impl HttpBody {
    /// The whole body, once it has all arrived by `deadline`.
    pub(super) fn collect(self, deadline: Instant) -> Result<Vec<u8>, String> {
        let stream = match self {
            Self::Buffered(body) => return Ok(body),
            Self::Stream(stream) => stream,
        };
        let mut body = Vec::new();
        loop {
            match stream.receiver.recv_deadline(deadline) {
                Ok(chunk) => body.extend(chunk?),
                Err(RecvTimeoutError::Disconnected) => return Ok(body),
                Err(RecvTimeoutError::Timeout) => return Err("the body timed out".to_owned()),
            }
        }
    }
}

impl Drop for BodyStream {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

/// A protocol violation described by `message`.
pub(super) fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn stream_failure(error: &str) -> io::Error {
    io::Error::other(format!("response stream failed: {error}"))
}

pub(super) fn read_header_block(
    stream: &mut impl Read,
    subject: &'static str,
) -> io::Result<Vec<u8>> {
    read_optional_header_block(stream, subject)?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
}

/// Reads a header block, or `None` when the connection ends or goes idle before one begins.
fn read_optional_header_block(
    stream: &mut impl Read,
    subject: &'static str,
) -> io::Result<Option<Vec<u8>>> {
    let mut first = [0; 1];
    match stream.read(&mut first) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) if connection_ended(&error) => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut data = first.to_vec();
    loop {
        if data.ends_with(b"\r\n\r\n") {
            return Ok(Some(data));
        }
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        data.push(byte[0]);
        if data.len() > MAX_HEADERS {
            return Err(invalid_data(&format!("{subject} exceed the limit")));
        }
    }
}

/// Whether `error` means the client closed the connection, reset it, stopped
/// responding, or ended the TLS session or refused the gateway's certificate
/// with an alert, as Android's `WebView` does before asking the app to trust
/// it. The shells log a certificate the app refuses, and the gateway raises
/// its own failures with other kinds.
pub(super) fn connection_ended(error: &io::Error) -> bool {
    let alerted = matches!(
        error.get_ref().and_then(|inner| inner.downcast_ref()),
        Some(rustls::Error::AlertReceived(
            AlertDescription::CloseNotify
                | AlertDescription::UserCanceled
                | AlertDescription::CertificateUnknown
                | AlertDescription::BadCertificate
                | AlertDescription::UnsupportedCertificate
                | AlertDescription::CertificateExpired
                | AlertDescription::CertificateRevoked
                | AlertDescription::UnknownCA
        ))
    );
    alerted
        || matches!(
            error.kind(),
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::TimedOut
                | io::ErrorKind::WouldBlock
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::NotConnected
        )
}

pub(super) fn is_connect(data: &[u8], host: &str) -> bool {
    let text = String::from_utf8_lossy(data);
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    parts.next() == Some("CONNECT")
        && parts
            .next()
            .is_some_and(|target| target.eq_ignore_ascii_case(&format!("{host}:443")))
}

/// Completes the TLS handshake for an accepted gateway connection.
pub(super) fn tls_accept(config: Arc<ServerConfig>, stream: TcpStream) -> io::Result<TlsStream> {
    let connection = ServerConnection::new(config)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut stream = StreamOwned::new(connection, stream);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    Ok(stream)
}

/// Ends the TLS session cleanly so the peer sees a close rather than an abrupt disconnect.
pub(super) fn tls_close(stream: &mut TlsStream) -> io::Result<()> {
    stream.conn.send_close_notify();
    match stream.conn.complete_io(&mut stream.sock) {
        Ok(_) => Ok(()),
        Err(error) if connection_ended(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

/// Reads the next request, or `None` when the client closes the connection or goes idle first.
pub(super) fn read_request(stream: &mut impl Read, host: &str) -> io::Result<Option<HttpRequest>> {
    let Some(headers) = read_optional_header_block(stream, "HTTP headers")? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&headers);
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let Some(method) = request_parts.next() else {
        return Err(invalid_data("HTTP method is missing"));
    };
    let target = request_parts.next().unwrap_or("/");
    let version = request_parts.next().unwrap_or("HTTP/1.1");
    let (mut request_headers, content_length, chunked) = parse_header_fields(lines)?;
    if chunked && content_length.is_some() {
        return Err(invalid_data(
            "chunked request cannot include content length",
        ));
    }
    if content_length.is_some_and(|length| length > MAX_HTTP_BODY) {
        return Err(invalid_data("HTTP body exceeds the limit"));
    }
    let body = if chunked {
        read_chunked_body(stream)?
    } else {
        let mut body = vec![0; content_length.unwrap_or_default()];
        stream.read_exact(&mut body)?;
        body
    };
    let body = (!body.is_empty()).then_some(body);
    let persistent = connection_persists(version, &request_headers);
    // Workers observe what Cloudflare's edge shows them: the connection is always kept alive.
    request_headers.insert("connection", HeaderValue::from_static("Keep-Alive"));
    Ok(Some(HttpRequest {
        persistent,
        method: method.to_owned(),
        target: target.to_owned(),
        url: format!("https://{host}{target}"),
        headers: request_headers,
        body,
    }))
}

/// The header fields before the first empty line, the declared content
/// length, and whether the body is chunked.
fn parse_header_fields<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> io::Result<(HeaderMap, Option<usize>, bool)> {
    let mut headers = HeaderMap::new();
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim_matches([' ', '\t']);
        if name == "content-length" {
            content_length = Some(
                value
                    .parse()
                    .map_err(|_| invalid_data("invalid content length"))?,
            );
        }
        if name == "transfer-encoding" && has_token(value, "chunked") {
            chunked = true;
        }
        append_header(&mut headers, &name, value)?;
    }
    Ok((headers, content_length, chunked))
}

/// Appends a field, failing when the name or value is invalid or the map is full.
pub(super) fn append_header(headers: &mut HeaderMap, name: &str, value: &str) -> io::Result<()> {
    headers
        .try_append(
            HeaderName::from_bytes(name.as_bytes()).map_err(io::Error::other)?,
            HeaderValue::from_bytes(value.as_bytes()).map_err(io::Error::other)?,
        )
        .map(drop)
        .map_err(io::Error::other)
}

/// Inserts a field, failing when the map is full.
fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: HeaderValue,
) -> io::Result<()> {
    headers
        .try_insert(name, value)
        .map(drop)
        .map_err(io::Error::other)
}

fn read_chunked_body(stream: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut trailer_bytes = 0usize;
    loop {
        let line = read_line(stream)?;
        let size = line.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size, 16).map_err(|_| invalid_data("chunk size is invalid"))?;
        if size == 0 {
            loop {
                let trailer = read_line(stream)?;
                trailer_bytes = trailer_bytes.saturating_add(trailer.len() + 2);
                if trailer_bytes > MAX_HEADERS {
                    return Err(invalid_data("HTTP trailers exceed the limit"));
                }
                if trailer.is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len().saturating_add(size) > MAX_HTTP_BODY {
            return Err(invalid_data("HTTP body exceeds the limit"));
        }
        let start = body.len();
        body.resize(start + size, 0);
        stream.read_exact(&mut body[start..])?;
        let mut terminator = [0; 2];
        stream.read_exact(&mut terminator)?;
        if terminator != *b"\r\n" {
            return Err(invalid_data("chunk is not terminated"));
        }
    }
}

fn read_line(stream: &mut impl Read) -> io::Result<String> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
        if line.len() > MAX_HEADERS {
            return Err(invalid_data("HTTP line is too long"));
        }
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).map_err(|_| invalid_data("HTTP line is not UTF-8"));
        }
    }
}

pub(super) fn is_websocket(request: &HttpRequest) -> bool {
    request
        .headers
        .get("upgrade")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
}

pub(super) fn websocket_session(
    stream: &mut TlsStream,
    key: Option<&str>,
    bridge: WebSocketBridge,
) -> io::Result<()> {
    let key = key.ok_or_else(|| invalid_data("WebSocket key is missing"))?;
    let accept = websocket_accept(key);
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    stream.flush()?;
    let WebSocketBridge {
        incoming,
        outgoing,
        mut readiness,
    } = bridge;
    readiness.watch(stream.get_ref())?;
    let mut codec = WebSocketCodec::new(ReadySocket { stream, readiness });
    wait_for_websocket_ready(&mut codec, &outgoing)?;

    // After the handler's close frame, the session reads until the client's
    // close frame answers it.
    let mut fragmented: Option<(u8, Vec<u8>)> = None;
    loop {
        flush_websocket_outbound(&mut codec, &outgoing)?;
        match codec.read_frame()? {
            WebSocketRead::Closed => return Ok(()),
            WebSocketRead::Pending => codec.stream.readiness.wait(None)?,
            WebSocketRead::Frame(frame) => match frame.opcode {
                0x8 => {
                    let (code, reason) = websocket_close(&frame.payload)?;
                    codec.write_frame(frame.opcode, &frame.payload)?;
                    let _ = incoming.send(WebSocketInbound::Close { code, reason });
                    return Ok(());
                }
                0x9 => codec.write_frame(0xA, &frame.payload)?,
                0xA => {}
                0x0..=0x2 => queue_websocket_message(
                    &incoming,
                    &mut fragmented,
                    frame.final_frame,
                    frame.opcode,
                    frame.payload,
                )?,
                _ => return Err(invalid_data("invalid WebSocket opcode")),
            },
        }
    }
}

/// Non-blocking socket whose writes wait for readiness instead of failing.
struct ReadySocket<S> {
    stream: S,
    readiness: Readiness,
}

impl<S: Write> ReadySocket<S> {
    // rustls buffers the written record, so only flushing it to the socket can block.
    fn until_ready<T>(
        &mut self,
        mut operation: impl FnMut(&mut S) -> io::Result<T>,
    ) -> io::Result<T> {
        let deadline = Instant::now() + WEBSOCKET_WRITE_TIMEOUT;
        loop {
            match operation(&mut self.stream) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.readiness.wait_writable(deadline)?;
                }
                result => return result,
            }
        }
    }
}

impl<S: Read> Read for ReadySocket<S> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buffer)
    }
}

impl<S: Write> Write for ReadySocket<S> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.until_ready(|stream| stream.write(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.until_ready(Write::flush)
    }
}

enum WebSocketRead {
    Frame(WebSocketFrame),
    Pending,
    Closed,
}

/// The server side of a WebSocket connection to the `WebView`.
struct WebSocketCodec<S> {
    stream: S,
    buffer: Vec<u8>,
    closed: bool,
    close_sent: bool,
}

impl<S: Read + Write> WebSocketCodec<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            closed: false,
            close_sent: false,
        }
    }

    fn read_frame(&mut self) -> io::Result<WebSocketRead> {
        if self.closed {
            return Ok(WebSocketRead::Closed);
        }
        loop {
            if let Some(frame) = parse_websocket_frame(&mut self.buffer, true)? {
                return Ok(WebSocketRead::Frame(frame));
            }
            let mut bytes = [0; 8192];
            match self.stream.read(&mut bytes) {
                Ok(0) => {
                    self.closed = true;
                    return Ok(WebSocketRead::Closed);
                }
                Ok(count) => self.buffer.extend_from_slice(&bytes[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(WebSocketRead::Pending);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Writes a frame, or nothing once the session has sent its close frame.
    fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        if self.close_sent {
            return Ok(());
        }
        self.close_sent = opcode == 0x8;
        self.stream
            .write_all(&encode_websocket_frame(opcode, payload, false)?)?;
        self.stream.flush()
    }
}

/// A final frame carrying `payload`, masked with a random key when `mask`.
pub(super) fn encode_websocket_frame(
    opcode: u8,
    payload: &[u8],
    mask: bool,
) -> io::Result<Vec<u8>> {
    if payload.len() > MAX_WEBSOCKET_BODY {
        return Err(invalid_data("WebSocket frame is too large"));
    }
    if opcode >= 0x8 && (payload.len() > 125 || opcode & 0x40 != 0) {
        return Err(invalid_data("WebSocket control frame is invalid"));
    }
    let mask_bit = if mask { 0x80 } else { 0 };
    let mut frame = vec![0x80 | opcode];
    match (u8::try_from(payload.len()), u16::try_from(payload.len())) {
        (Ok(length @ 0..=125), _) => frame.push(mask_bit | length),
        (_, Ok(length)) => {
            frame.push(mask_bit | 0x7e);
            frame.extend_from_slice(&length.to_be_bytes());
        }
        _ => {
            frame.push(mask_bit | 0x7f);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
    }
    if !mask {
        frame.extend_from_slice(payload);
        return Ok(frame);
    }
    let mut key = [0; 4];
    getrandom::fill(&mut key)
        .map_err(|error| io::Error::other(format!("WebSocket mask failed: {error}")))?;
    frame.extend_from_slice(&key);
    let start = frame.len();
    frame.extend_from_slice(payload);
    apply_mask(&mut frame[start..], &key);
    Ok(frame)
}

/// Writes a masked frame, as a client sends one, to `writer`.
pub(super) async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    opcode: u8,
    payload: &[u8],
) -> io::Result<()> {
    let frame = encode_websocket_frame(opcode, payload, true)?;
    tokio::time::timeout(WEBSOCKET_WRITE_TIMEOUT, async {
        writer.write_all(&frame).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "WebSocket write timed out"))?
}

/// Removes the first complete frame from `buffer`, failing when its masking
/// is not `expect_mask` or it breaks the protocol.
pub(super) fn parse_websocket_frame(
    buffer: &mut Vec<u8>,
    expect_mask: bool,
) -> io::Result<Option<WebSocketFrame>> {
    if buffer.len() > MAX_WEBSOCKET_BODY + 14 {
        return Err(invalid_data("WebSocket frame is too large"));
    }
    let Some(header) = parse_websocket_header(buffer, expect_mask)? else {
        return Ok(None);
    };
    if buffer.len() < header.frame_length {
        return Ok(None);
    }
    let mut payload = buffer[header.payload_offset..header.frame_length].to_vec();
    if let Some(offset) = header.mask_offset {
        apply_mask(&mut payload, &buffer[offset..offset + 4]);
    }
    buffer.drain(..header.frame_length);
    Ok(Some(WebSocketFrame {
        final_frame: header.final_frame,
        opcode: header.opcode,
        payload,
    }))
}

/// XORs `payload` with the repeating `key`.
fn apply_mask(payload: &mut [u8], key: &[u8]) {
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= key[index % key.len()];
    }
}

struct WebSocketHeader {
    final_frame: bool,
    opcode: u8,
    payload_offset: usize,
    frame_length: usize,
    mask_offset: Option<usize>,
}

fn parse_websocket_header(buffer: &[u8], expect_mask: bool) -> io::Result<Option<WebSocketHeader>> {
    if buffer.len() < 2 {
        return Ok(None);
    }
    let first = buffer[0];
    let second = buffer[1];
    if first & 0x70 != 0 {
        return Err(invalid_data("WebSocket reserved bits are unsupported"));
    }
    let masked = second & 0x80 != 0;
    if masked != expect_mask {
        let message = if expect_mask {
            "client WebSocket frames must be masked"
        } else {
            "server WebSocket frames must not be masked"
        };
        return Err(invalid_data(message));
    }
    let mut offset = 2;
    let length = match second & 0x7f {
        length @ 0..=125 => usize::from(length),
        126 => {
            let Some(bytes) = buffer[offset..].first_chunk::<2>() else {
                return Ok(None);
            };
            let length = u16::from_be_bytes(*bytes);
            offset += 2;
            usize::from(length)
        }
        127 => {
            let Some(bytes) = buffer[offset..].first_chunk::<8>() else {
                return Ok(None);
            };
            let length = u64::from_be_bytes(*bytes);
            offset += 8;
            usize::try_from(length).map_err(|_| invalid_data("WebSocket frame is too large"))?
        }
        _ => unreachable!(),
    };
    if length > MAX_WEBSOCKET_BODY {
        return Err(invalid_data("WebSocket frame is too large"));
    }
    let opcode = first & 0x0f;
    let final_frame = first & 0x80 != 0;
    if opcode >= 0x8 && (!final_frame || length > 125) {
        return Err(invalid_data("WebSocket control frame is invalid"));
    }
    let mask_offset = masked.then_some(offset);
    if masked {
        offset += 4;
    }
    let frame_length = offset
        .checked_add(length)
        .ok_or_else(|| invalid_data("WebSocket frame is too large"))?;
    Ok(Some(WebSocketHeader {
        final_frame,
        opcode,
        payload_offset: offset,
        frame_length,
        mask_offset,
    }))
}

/// Where a session delivers each whole message, which it drops once the
/// other end has gone.
pub(super) trait WebSocketSink {
    fn deliver(&self, binary: bool, payload: Vec<u8>);
}

impl WebSocketSink for Sender<WebSocketInbound> {
    fn deliver(&self, binary: bool, payload: Vec<u8>) {
        let _ = self.send(WebSocketInbound::Message { binary, payload });
    }
}

impl WebSocketSink for WebSocketOutgoing {
    fn deliver(&self, binary: bool, payload: Vec<u8>) {
        self.send(WebSocketOutbound::Message { binary, payload });
    }
}

pub(super) fn queue_websocket_message(
    sink: &impl WebSocketSink,
    fragmented: &mut Option<(u8, Vec<u8>)>,
    final_frame: bool,
    opcode: u8,
    payload: Vec<u8>,
) -> io::Result<()> {
    let (message_opcode, payload) = match opcode {
        0x0 => {
            let Some((initial_opcode, mut message)) = fragmented.take() else {
                return Err(invalid_data("WebSocket continuation has no initial frame"));
            };
            if message.len().saturating_add(payload.len()) > MAX_WEBSOCKET_BODY {
                return Err(invalid_data("WebSocket message is too large"));
            }
            message.extend_from_slice(&payload);
            (initial_opcode, message)
        }
        0x1 | 0x2 => {
            if fragmented.is_some() {
                return Err(invalid_data(
                    "WebSocket message starts before the previous message ended",
                ));
            }
            (opcode, payload)
        }
        _ => return Err(invalid_data("invalid WebSocket opcode")),
    };
    if final_frame {
        sink.deliver(message_opcode == 0x2, payload);
    } else {
        *fragmented = Some((message_opcode, payload));
    }
    Ok(())
}

/// Writes the handler's queued frames. A handler that ends without closing the
/// connection has failed.
fn flush_websocket_outbound<S: Read + Write>(
    codec: &mut WebSocketCodec<S>,
    outgoing: &Receiver<WebSocketOutbound>,
) -> io::Result<()> {
    loop {
        let frame = match outgoing.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) if codec.close_sent => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                return Err(io::Error::other("WebSocket worker closed"));
            }
        };
        write_outbound(codec, frame)?;
    }
}

/// Writes the frames the handler queues until it is ready to receive.
fn wait_for_websocket_ready<S: Read + Write>(
    codec: &mut WebSocketCodec<S>,
    outgoing: &Receiver<WebSocketOutbound>,
) -> io::Result<()> {
    loop {
        match outgoing.recv_timeout(Duration::from_secs(30)) {
            Ok(WebSocketOutbound::Ready) => return Ok(()),
            Ok(frame) => write_outbound(codec, frame)?,
            Err(_) => return Err(io::Error::other("WebSocket worker did not become ready")),
        }
    }
}

/// Writes a frame the handler queued.
fn write_outbound<S: Read + Write>(
    codec: &mut WebSocketCodec<S>,
    frame: WebSocketOutbound,
) -> io::Result<()> {
    let (opcode, payload) = match frame {
        WebSocketOutbound::Message { binary, payload } => (if binary { 0x2 } else { 0x1 }, payload),
        WebSocketOutbound::Close { code, reason } => (0x8, websocket_close_payload(code, &reason)?),
        WebSocketOutbound::Ready => return Ok(()),
    };
    codec.write_frame(opcode, &payload)
}

pub(super) fn websocket_close(payload: &[u8]) -> io::Result<(u16, String)> {
    if payload.is_empty() {
        return Ok((1000, String::new()));
    }
    let Some((code, reason)) = payload.split_first_chunk() else {
        return Err(invalid_data("WebSocket close payload is invalid"));
    };
    let code = u16::from_be_bytes(*code);
    if !valid_websocket_close_code(code) {
        return Err(invalid_data("WebSocket close code is invalid"));
    }
    let reason = std::str::from_utf8(reason)
        .map_err(|_| invalid_data("WebSocket close reason is invalid"))?
        .to_owned();
    Ok((code, reason))
}

fn valid_websocket_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

pub(super) fn websocket_close_payload(code: u16, reason: &str) -> io::Result<Vec<u8>> {
    if !valid_websocket_close_code(code) {
        return Err(invalid_data("WebSocket close code is invalid"));
    }
    if reason.len() > 123 {
        return Err(invalid_data("WebSocket close reason is too long"));
    }
    let mut payload = Vec::with_capacity(2 + reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(reason.as_bytes());
    Ok(payload)
}

pub(super) fn websocket_accept(key: &str) -> String {
    let digest = Sha1::digest(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    STANDARD.encode(digest)
}

pub(super) fn write_plain_response(
    stream: &mut TcpStream,
    response: HttpResponse,
) -> io::Result<()> {
    write_response_inner(stream, response, None, None, |_| Ok(false), false).map(|_| ())
}

/// HTTP/1.1 connections persist unless the client says `close`; HTTP/1.0 only with `keep-alive`.
fn connection_persists(version: &str, headers: &HeaderMap) -> bool {
    let connection = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if version.eq_ignore_ascii_case("HTTP/1.0") {
        has_token(connection, "keep-alive")
    } else {
        !has_token(connection, "close")
    }
}

/// Whether a comma-separated header value contains `token`, ignoring case and whitespace.
fn has_token(value: &str, token: &str) -> bool {
    value
        .split(',')
        .any(|candidate| candidate.trim().eq_ignore_ascii_case(token))
}

/// Whether a response reached the client in full or the connection was lost part way.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResponseOutcome {
    Complete,
    Interrupted,
}

pub(super) fn write_response(
    stream: &mut TlsStream,
    response: HttpResponse,
    method: &str,
    cancelled: &AtomicBool,
    persistent: bool,
) -> io::Result<ResponseOutcome> {
    let mut readiness = match response.body {
        HttpBody::Stream(_) => {
            let (mut readiness, _wake) = Readiness::new()?;
            readiness.observe(stream.get_ref())?;
            stream
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(1)))?;
            Some(readiness)
        }
        HttpBody::Buffered(_) => None,
    };
    write_response_inner(
        stream,
        response,
        Some(method),
        Some(cancelled),
        |stream| match readiness.as_mut() {
            Some(readiness) => client_disconnected(stream, readiness),
            None => Ok(false),
        },
        persistent,
    )
}

// The gateway owns connection lifetime; worker-set connection headers are ignored.
fn write_response_inner<W: Write>(
    mut stream: W,
    mut response: HttpResponse,
    method: Option<&str>,
    cancelled: Option<&AtomicBool>,
    peer_closed: impl FnMut(&mut W) -> io::Result<bool>,
    persistent: bool,
) -> io::Result<ResponseOutcome> {
    let head = method.is_some_and(|method| method.eq_ignore_ascii_case("HEAD"));
    let no_body =
        head || (100..200).contains(&response.status) || matches!(response.status, 204 | 205 | 304);
    let headers = &mut response.headers;
    headers.remove("keep-alive");
    let connection = if persistent { "keep-alive" } else { "close" };
    insert_header(headers, "connection", HeaderValue::from_static(connection))?;
    insert_framing_headers(headers, response.status, &response.body, head)?;
    write_response_headers(
        &mut stream,
        response.status,
        &response.status_text,
        &response.headers,
    )?;
    if no_body {
        return Ok(ResponseOutcome::Complete);
    }
    match response.body {
        HttpBody::Buffered(body) => {
            stream.write_all(&body)?;
            stream.flush()?;
            Ok(ResponseOutcome::Complete)
        }
        HttpBody::Stream(body) => write_stream_body(stream, &body, cancelled, peer_closed),
    }
}

/// Sets the length headers of a `status` response with `body`, or of its head
/// alone when `head`.
fn insert_framing_headers(
    headers: &mut HeaderMap,
    status: u16,
    body: &HttpBody,
    head: bool,
) -> io::Result<()> {
    if head {
        if status == 205
            && !headers.contains_key("content-length")
            && !headers.contains_key("transfer-encoding")
        {
            insert_header(headers, "content-length", HeaderValue::from_static("0"))?;
        }
        return Ok(());
    }
    headers.remove("transfer-encoding");
    if (100..200).contains(&status) || matches!(status, 204 | 304) {
        headers.remove("content-length");
    } else if let HttpBody::Buffered(body) = body {
        insert_header(headers, "content-length", HeaderValue::from(body.len()))?;
    } else {
        headers.remove("content-length");
        insert_header(
            headers,
            "transfer-encoding",
            HeaderValue::from_static("chunked"),
        )?;
    }
    Ok(())
}

fn write_stream_body<W: Write>(
    mut stream: W,
    body: &BodyStream,
    cancelled: Option<&AtomicBool>,
    mut peer_closed: impl FnMut(&mut W) -> io::Result<bool>,
) -> io::Result<ResponseOutcome> {
    let is_cancelled = || {
        cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
            || body.cancelled.is_cancelled()
    };
    loop {
        let chunk = match body.receiver.recv_timeout(RESPONSE_STREAM_POLL) {
            Ok(chunk) => chunk.map_err(|error| stream_failure(&error))?,
            Err(flume::RecvTimeoutError::Timeout) => {
                if is_cancelled() {
                    break;
                }
                if peer_closed(&mut stream)? {
                    body.cancelled.cancel();
                    return Ok(ResponseOutcome::Interrupted);
                }
                continue;
            }
            Err(flume::RecvTimeoutError::Disconnected) => break,
        };
        if !chunk.is_empty() {
            write!(stream, "{:X}\r\n", chunk.len())?;
            stream.write_all(&chunk)?;
            stream.write_all(b"\r\n")?;
            stream.flush()?;
        }
    }
    if is_cancelled() {
        return Ok(ResponseOutcome::Interrupted);
    }
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()?;
    Ok(ResponseOutcome::Complete)
}

fn write_response_headers(
    stream: &mut impl Write,
    status: u16,
    reason: &str,
    headers: &HeaderMap,
) -> io::Result<()> {
    hyper::ext::ReasonPhrase::try_from(reason.as_bytes()).map_err(io::Error::other)?;
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: ")?;
        stream.write_all(value.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn bounds_writes_to_a_peer_that_stops_reading() -> Result<(), Box<dyn std::error::Error>>
    {
        let (mut writer, _peer) = tokio::io::duplex(1);
        let result = tokio::time::timeout(
            super::WEBSOCKET_WRITE_TIMEOUT + std::time::Duration::from_secs(1),
            super::write_frame(&mut writer, 0x2, b"payload"),
        )
        .await?;
        assert_eq!(
            result.err().ok_or("the blocked write succeeded")?.kind(),
            std::io::ErrorKind::TimedOut
        );
        Ok(())
    }

    use super::{
        HttpBody, HttpResponse, ResponseOutcome, connection_ended, read_request, response_stream,
        write_response_inner,
    };
    use hyper::header::{HeaderMap, HeaderName, HeaderValue};
    use rustls::AlertDescription;
    use std::io::{self, Cursor, Read};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn counts_a_certificate_refusal_but_not_a_protocol_alert_as_the_client_leaving() {
        let alert = |description| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                rustls::Error::AlertReceived(description),
            )
        };
        assert!(connection_ended(&alert(
            AlertDescription::CertificateUnknown
        )));
        assert!(connection_ended(&alert(AlertDescription::CloseNotify)));
        assert!(!connection_ended(&alert(AlertDescription::DecodeError)));
        assert!(!connection_ended(&alert(AlertDescription::BadRecordMac)));
    }

    #[test]
    fn replaces_worker_connection_headers_with_the_gateway_decision()
    -> Result<(), Box<dyn std::error::Error>> {
        for (persistent, expected) in [
            (true, "connection: keep-alive"),
            (false, "connection: close"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("connection", HeaderValue::from_static("close"));
            headers.insert("keep-alive", HeaderValue::from_static("timeout=5"));
            let response = HttpResponse::buffered(200, headers, b"ok".to_vec());
            let mut output = Vec::new();
            write_response_inner(&mut output, response, None, None, |_| Ok(false), persistent)?;
            let text = String::from_utf8(output)?;
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains("keep-alive:"), "{text}");
        }
        Ok(())
    }

    #[test]
    fn decides_persistence_the_way_the_edge_does() -> Result<(), Box<dyn std::error::Error>> {
        for (request, persistent) in [
            ("GET / HTTP/1.1\r\nHost: app.test\r\n\r\n", true),
            (
                "GET / HTTP/1.1\r\nHost: app.test\r\nConnection: keep-alive, Close\r\n\r\n",
                false,
            ),
            ("GET / HTTP/1.0\r\nHost: app.test\r\n\r\n", false),
            (
                "GET / HTTP/1.0\r\nHost: app.test\r\nConnection: Keep-Alive\r\n\r\n",
                true,
            ),
        ] {
            let parsed = read_request(&mut Cursor::new(request.as_bytes().to_vec()), "app.test")?
                .ok_or("request was empty")?;
            assert_eq!(parsed.persistent, persistent, "{request:?}");
            assert_eq!(
                parsed.headers.get("connection").map(HeaderValue::as_bytes),
                Some(&b"Keep-Alive"[..])
            );
        }
        Ok(())
    }

    #[test]
    fn treats_an_idle_timeout_as_no_request() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let _client = TcpStream::connect(listener.local_addr()?)?;
        let (mut server, _) = listener.accept()?;
        server.set_read_timeout(Some(Duration::from_millis(10)))?;
        assert!(read_request(&mut server, "app.test")?.is_none());
        Ok(())
    }

    #[test]
    fn treats_a_closed_idle_connection_as_no_request() -> Result<(), Box<dyn std::error::Error>> {
        assert!(read_request(&mut Cursor::new(Vec::new()), "app.test")?.is_none());
        let mut input = Cursor::new(b"GET /path HTTP/1.1\r\nHost: app.test\r\n\r\n".to_vec());
        assert!(read_request(&mut input, "app.test")?.is_some());
        Ok(())
    }

    /// A response with as many headers as a header map holds, including a
    /// `connection` header unless `scenario` is 0, with a stream body when it is 2.
    fn excess_headers_response(
        scenario: u8,
    ) -> Result<(HttpResponse, Option<CancellationToken>), Box<dyn std::error::Error>> {
        let mut headers = HeaderMap::new();
        if scenario != 0 {
            headers.insert("connection", HeaderValue::from_static("close"));
        }
        for index in 0..30000 {
            let name = HeaderName::from_bytes(format!("x-{index}").as_bytes())?;
            if headers
                .try_insert(name, HeaderValue::from_static("v"))
                .is_err()
            {
                break;
            }
        }
        let (body, cancelled) = if scenario == 2 {
            let (_, cancelled, body) = response_stream();
            (body, Some(cancelled))
        } else {
            (HttpBody::Buffered(Vec::new()), None)
        };
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers,
            body,
        };
        Ok((response, cancelled))
    }

    #[test]
    fn rejects_excess_response_headers_without_panicking() -> Result<(), Box<dyn std::error::Error>>
    {
        for scenario in 0..3 {
            let (response, cancelled) = excess_headers_response(scenario)?;
            let mut output = Vec::new();
            assert!(
                write_response_inner(&mut output, response, None, None, |_| Ok(false), false)
                    .is_err()
            );
            assert!(output.is_empty());
            assert!(cancelled.is_none_or(|token| token.is_cancelled()));
        }
        Ok(())
    }

    #[test]
    fn reads_and_serializes_repeated_request_headers() -> Result<(), Box<dyn std::error::Error>> {
        let mut input = Cursor::new(
            "GET / HTTP/1.1\r\nX-Repeated: one\r\nX-Repeated: two\r\nX-Text: 東京\r\n\r\n",
        );
        let request = read_request(&mut input, "app.test")?.ok_or("request was empty")?;
        let json = serde_json::to_value(request)?;
        let pairs = json["headers"].as_array().ok_or("expected header pairs")?;
        assert!(pairs.contains(&serde_json::json!(["x-repeated", "one"])));
        assert!(pairs.contains(&serde_json::json!(["x-repeated", "two"])));
        assert!(pairs.contains(&serde_json::json!(["x-text", "東京"])));
        Ok(())
    }

    #[test]
    fn writes_custom_status_text_without_allowing_header_injection()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = HttpResponse::text(201, "ok");
        response.status_text = "Created Here".to_owned();
        let mut output = Vec::new();
        write_response_inner(&mut output, response, None, None, |_| Ok(false), false)?;
        assert!(output.starts_with(b"HTTP/1.1 201 Created Here\r\n"));

        let mut response = HttpResponse::text(200, "ok");
        response.status_text = "OK\r\nInjected: true".to_owned();
        let mut output = Vec::new();
        assert!(
            write_response_inner(&mut output, response, None, None, |_| Ok(false), false).is_err()
        );
        assert!(output.is_empty());
        Ok(())
    }

    #[test]
    fn writes_duplicate_response_headers() -> Result<(), Box<dyn std::error::Error>> {
        let headers = [
            (
                HeaderName::from_static("set-cookie"),
                HeaderValue::from_static("first=1"),
            ),
            (
                HeaderName::from_static("set-cookie"),
                HeaderValue::from_static("second=2"),
            ),
        ]
        .into_iter()
        .collect();
        let response = HttpResponse::buffered(201, headers, b"ok".to_vec());
        let mut output = Vec::new();
        write_response_inner(&mut output, response, None, None, |_| Ok(false), false)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("set-cookie: first=1\r\n"));
        assert!(output.contains("set-cookie: second=2\r\n"));
        assert!(output.starts_with("HTTP/1.1 201 Created\r\n"));
        Ok(())
    }

    /// Writes `response` with `headers` set, for a `method` request, and asserts
    /// whether each probe appears in the output.
    fn assert_framing(
        mut response: HttpResponse,
        method: &str,
        headers: &[(&'static str, &'static str)],
        probes: &[(&str, bool)],
    ) -> Result<(), Box<dyn std::error::Error>> {
        for &(name, value) in headers {
            response
                .headers
                .insert(name, HeaderValue::from_static(value));
        }
        let mut output = Vec::new();
        write_response_inner(
            &mut output,
            response,
            Some(method),
            None,
            |_| Ok(false),
            false,
        )?;
        let output = String::from_utf8(output)?;
        for &(probe, present) in probes {
            assert_eq!(output.contains(probe), present, "{probe:?} in {output}");
        }
        Ok(())
    }

    #[test]
    fn computes_response_framing_and_preserves_explicit_head_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_framing(
            HttpResponse::text(200, "ok"),
            "GET",
            &[("content-length", "99"), ("transfer-encoding", "identity")],
            &[
                ("content-length: 2\r\n", true),
                ("transfer-encoding:", false),
            ],
        )?;
        for explicit in [false, true] {
            let (_, cancelled, body) = response_stream();
            let mut response = HttpResponse::text(200, "");
            response.body = body;
            if explicit {
                response
                    .headers
                    .insert("content-length", HeaderValue::from_static("99"));
            }
            assert_framing(
                response,
                "HEAD",
                &[],
                &[
                    ("content-length: 99\r\n", explicit),
                    ("transfer-encoding:", false),
                ],
            )?;
            assert!(cancelled.is_cancelled());
        }
        for status in [204, 205, 304] {
            assert_framing(
                HttpResponse::text(status, ""),
                "GET",
                &[("content-length", "99")],
                &[
                    ("content-length: 0\r\n", status == 205),
                    ("content-length: 99", false),
                    ("transfer-encoding:", false),
                ],
            )?;
        }
        Ok(())
    }

    #[test]
    fn writes_buffered_responses_with_a_content_length() -> Result<(), Box<dyn std::error::Error>> {
        let response = HttpResponse::buffered(200, HeaderMap::new(), b"ok".to_vec());
        let mut output = Vec::new();

        write_response_inner(&mut output, response, None, None, |_| Ok(false), false)?;

        let output = String::from_utf8(output)?;
        assert!(output.contains("content-length: 2\r\n"));
        assert!(output.ends_with("\r\nok"));
        Ok(())
    }

    #[test]
    fn writes_streamed_responses_as_chunked_data() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, _cancelled, body) = response_stream();
        let HttpBody::Stream(body) = body else {
            return Err("response stream fixture was buffered".into());
        };
        sender.send(Ok(b"one".to_vec()))?;
        sender.send(Ok(b"two".to_vec()))?;
        drop(sender);
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers: HeaderMap::new(),
            body: HttpBody::Stream(body),
        };
        let mut output = Vec::new();

        write_response_inner(&mut output, response, None, None, |_| Ok(false), false)?;

        let output = String::from_utf8(output)?;
        assert!(output.contains("transfer-encoding: chunked\r\n"));
        assert!(!output.contains("content-length:"));
        assert!(output.ends_with("\r\n3\r\ntwo\r\n0\r\n\r\n"));
        Ok(())
    }

    #[test]
    fn suppresses_a_stream_body_for_head_requests() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, cancelled, body) = response_stream();
        let HttpBody::Stream(body) = body else {
            return Err("response stream fixture was buffered".into());
        };
        sender.send(Ok(b"body".to_vec()))?;
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers: HeaderMap::new(),
            body: HttpBody::Stream(body),
        };
        let mut output = Vec::new();

        write_response_inner(
            &mut output,
            response,
            Some("HEAD"),
            None,
            |_| Ok(false),
            false,
        )?;

        let output = String::from_utf8(output)?;
        assert!(!output.contains("transfer-encoding:"));
        assert!(!output.ends_with("body"));
        assert!(cancelled.is_cancelled());
        Ok(())
    }

    #[test]
    fn stops_an_idle_stream_when_the_gateway_stops() -> Result<(), Box<dyn std::error::Error>> {
        let (_sender, _cancelled, body) = response_stream();
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers: HeaderMap::new(),
            body,
        };
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let mut output = Vec::new();

        write_response_inner(
            &mut output,
            response,
            None,
            Some(&cancelled),
            |_| Ok(false),
            false,
        )?;

        assert!(String::from_utf8(output)?.contains("HTTP/1.1 200"));
        Ok(())
    }

    #[test]
    fn stops_an_idle_stream_when_the_client_disconnects() -> Result<(), Box<dyn std::error::Error>>
    {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let mut client = TcpStream::connect(address)?;
        let (server, _) = listener.accept()?;
        let peer = server.try_clone()?;
        peer.set_read_timeout(Some(Duration::from_millis(1)))?;
        let (sender, cancelled, body) = response_stream();
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers: HeaderMap::new(),
            body,
        };
        let (done_sender, done_receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            let result = write_response_inner(
                server,
                response,
                None,
                None,
                |_| Ok(matches!(peer.peek(&mut [0]), Ok(0))),
                false,
            );
            let _ = done_sender.send(result);
        });

        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0; 1];
            client.read_exact(&mut byte)?;
            headers.push(byte[0]);
        }
        drop(client);
        match done_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => assert_eq!(result?, ResponseOutcome::Interrupted),
            Err(error) => {
                cancelled.cancel();
                let _ = thread.join();
                return Err(error.into());
            }
        }
        thread.join().map_err(|_| "response writer panicked")?;
        assert!(cancelled.is_cancelled());
        drop(sender);
        Ok(())
    }

    #[test]
    fn accepts_the_configured_http_request_body_limit() {
        assert_eq!(super::MAX_HTTP_BODY, 250 * 1024 * 1024);
    }

    #[test]
    fn rejects_chunked_request_bodies_over_the_limit() {
        let input = format!("{:X}\r\n", super::MAX_HTTP_BODY + 1);
        assert!(matches!(
            super::read_chunked_body(&mut Cursor::new(input.as_bytes())),
            Err(error) if error.kind() == io::ErrorKind::InvalidData
                && error.to_string() == "HTTP body exceeds the limit"
        ));
    }

    #[test]
    fn reports_stream_errors_to_the_http_writer() -> Result<(), Box<dyn std::error::Error>> {
        let (sender, _cancelled, body) = response_stream();
        let HttpBody::Stream(body) = body else {
            return Err("response stream fixture was buffered".into());
        };
        sender.send(Err("broken".to_owned()))?;
        drop(sender);
        let response = HttpResponse {
            status: 200,
            status_text: "OK".to_owned(),
            headers: HeaderMap::new(),
            body: HttpBody::Stream(body),
        };
        let Err(error) =
            write_response_inner(io::sink(), response, None, None, |_| Ok(false), false)
        else {
            return Err("stream should fail".into());
        };
        assert_eq!(error.to_string(), "response stream failed: broken");
        Ok(())
    }
}
