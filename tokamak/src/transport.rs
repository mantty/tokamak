use flume::{Receiver, Sender, TryRecvError, bounded};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::sha::sha1;
use openssl::ssl::{SslAcceptor, SslMethod, SslStream, SslVerifyMode};
use openssl::x509::X509;
use reqwest::{
    StatusCode,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::gateway::{GatewayConfig, WebSocketBridge, WebSocketInbound, WebSocketOutbound};
use crate::quickjs::Error;

const MAX_HEADERS: usize = 64 * 1024;
pub(super) const MAX_HTTP_BODY: usize = 250 * 1024 * 1024;
const MAX_WEBSOCKET_BODY: usize = 16 * 1024 * 1024;
const RESPONSE_STREAM_QUEUE: usize = 8;
const RESPONSE_STREAM_POLL: Duration = Duration::from_millis(100);

pub(super) type BodyChunk = Result<Vec<u8>, String>;

#[derive(Serialize)]
pub(super) struct HttpRequest {
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

impl Drop for BodyStream {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

pub(super) fn read_headers(stream: &mut TcpStream) -> Result<Vec<u8>, Error> {
    read_header_block(stream)
}

fn read_header_block(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        data.push(byte[0]);
        if data.len() > MAX_HEADERS {
            return Err(Error::Startup("HTTP headers exceed the limit".to_owned()));
        }
        if data.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    Ok(data)
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

pub(super) fn tls_acceptor(config: &GatewayConfig) -> Result<SslAcceptor, Error> {
    let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls())
        .map_err(|error| tls_error(&error))?;
    let certificate = X509::from_pem(&std::fs::read(&config.certificates.certificate)?)
        .map_err(|error| tls_error(&error))?;
    let private_key = openssl::pkey::PKey::private_key_from_pem(&std::fs::read(
        &config.certificates.private_key,
    )?)
    .map_err(|error| tls_error(&error))?;
    builder
        .set_certificate(&certificate)
        .map_err(|error| tls_error(&error))?;
    builder
        .set_private_key(&private_key)
        .map_err(|error| tls_error(&error))?;
    builder
        .check_private_key()
        .map_err(|error| tls_error(&error))?;
    if config.require_client_certificate {
        let ca = X509::from_pem(&std::fs::read(&config.certificates.ca)?)
            .map_err(|error| tls_error(&error))?;
        builder
            .cert_store_mut()
            .add_cert(ca)
            .map_err(|error| tls_error(&error))?;
        builder.set_verify(SslVerifyMode::PEER);
    }
    Ok(builder.build())
}

pub(super) fn read_request(stream: &mut impl Read, host: &str) -> Result<HttpRequest, Error> {
    let headers = read_header_block(stream)?;
    let text = String::from_utf8_lossy(&headers);
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_owned();
    let target = request_parts.next().unwrap_or("/").to_owned();
    if method.is_empty() {
        return Err(Error::Startup("HTTP method is missing".to_owned()));
    }
    let mut request_headers = HeaderMap::new();
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
        let value = value.trim_matches([' ', '\t']).to_owned();
        if name == "content-length" {
            content_length = Some(
                value
                    .parse()
                    .map_err(|_| Error::Startup("invalid content length".to_owned()))?,
            );
        }
        if name == "transfer-encoding"
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
        request_headers
            .try_append(
                HeaderName::from_bytes(name.as_bytes()).map_err(io::Error::other)?,
                HeaderValue::from_bytes(value.as_bytes()).map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)?;
    }
    if chunked && content_length.is_some() {
        return Err(Error::Startup(
            "chunked request cannot include content length".to_owned(),
        ));
    }
    if content_length.is_some_and(|length| length > MAX_HTTP_BODY) {
        return Err(Error::Startup("HTTP body exceeds the limit".to_owned()));
    }
    let body = if chunked {
        read_chunked_body(stream)?
    } else {
        let mut body = vec![0; content_length.unwrap_or_default()];
        stream.read_exact(&mut body)?;
        body
    };
    let body = (!body.is_empty()).then_some(body);
    Ok(HttpRequest {
        method,
        target: target.clone(),
        url: format!("https://{host}{target}"),
        headers: request_headers,
        body,
    })
}

pub(super) fn read_chunked_body(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    let mut trailer_bytes = 0usize;
    loop {
        let line = read_line(stream)?;
        let size = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| Error::Startup("chunk size is invalid".to_owned()))?;
        if size == 0 {
            loop {
                let trailer = read_line(stream)?;
                trailer_bytes = trailer_bytes.saturating_add(trailer.len() + 2);
                if trailer_bytes > MAX_HEADERS {
                    return Err(Error::Startup("HTTP trailers exceed the limit".to_owned()));
                }
                if trailer.is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len().saturating_add(size) > MAX_HTTP_BODY {
            return Err(Error::Startup("HTTP body exceeds the limit".to_owned()));
        }
        let start = body.len();
        body.resize(start + size, 0);
        stream.read_exact(&mut body[start..])?;
        let mut terminator = [0; 2];
        stream.read_exact(&mut terminator)?;
        if terminator != *b"\r\n" {
            return Err(Error::Startup("chunk is not terminated".to_owned()));
        }
    }
}

fn read_line(stream: &mut impl Read) -> Result<String, Error> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
        if line.len() > MAX_HEADERS {
            return Err(Error::Startup("HTTP line is too long".to_owned()));
        }
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line)
                .map_err(|_| Error::Startup("HTTP line is not UTF-8".to_owned()));
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
    stream: &mut SslStream<TcpStream>,
    key: Option<&str>,
    bridge: &WebSocketBridge,
) -> Result<(), Error> {
    let key = key.ok_or_else(|| Error::Startup("WebSocket key is missing".to_owned()))?;
    let accept = websocket_accept(key);
    write!(
        stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    )?;
    stream.flush()?;
    stream
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))?;
    stream
        .get_mut()
        .set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut codec = WebSocketCodec::new(&mut *stream);
    if wait_for_websocket_ready(&mut codec, &bridge.outgoing)? {
        return Ok(());
    }

    let mut fragmented: Option<(u8, Vec<u8>)> = None;
    loop {
        if flush_websocket_outbound(&mut codec, &bridge.outgoing)? {
            return Ok(());
        }
        match codec.read_frame(true)? {
            WebSocketRead::Closed => return Ok(()),
            WebSocketRead::Pending => thread::sleep(Duration::from_millis(5)),
            WebSocketRead::Frame(frame) => match frame.opcode {
                0x8 => {
                    let (code, reason) = websocket_close(&frame.payload)?;
                    codec.write_frame(frame.opcode, &frame.payload, false)?;
                    let _ = bridge
                        .incoming
                        .send(WebSocketInbound::Close { code, reason });
                    return Ok(());
                }
                0x9 => codec.write_frame(0xA, &frame.payload, false)?,
                0xA => {}
                0x0..=0x2 => queue_websocket_message(
                    bridge,
                    &mut fragmented,
                    frame.final_frame,
                    frame.opcode,
                    frame.payload,
                )?,
                _ => return Err(Error::Startup("invalid WebSocket opcode".to_owned())),
            },
        }
    }
}

pub(super) enum WebSocketRead {
    Frame(WebSocketFrame),
    Pending,
    Closed,
}

pub(super) struct WebSocketCodec<S> {
    stream: S,
    buffer: Vec<u8>,
    closed: bool,
}

impl<S: Read + Write> WebSocketCodec<S> {
    pub(super) fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            closed: false,
        }
    }

    pub(super) fn read_frame(&mut self, expect_mask: bool) -> Result<WebSocketRead, Error> {
        if self.closed {
            return Ok(WebSocketRead::Closed);
        }
        loop {
            if let Some(frame) = parse_websocket_frame(&mut self.buffer, expect_mask)? {
                return Ok(WebSocketRead::Frame(frame));
            }
            let mut bytes = [0; 8192];
            match self.stream.read(&mut bytes) {
                Ok(0) => {
                    self.closed = true;
                    return Ok(WebSocketRead::Closed);
                }
                Ok(count) => {
                    self.buffer.extend_from_slice(&bytes[..count]);
                    if self.buffer.len() > MAX_WEBSOCKET_BODY + 14 {
                        return Err(Error::Startup("WebSocket frame is too large".to_owned()));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(WebSocketRead::Pending);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    pub(super) fn write_frame(
        &mut self,
        opcode: u8,
        payload: &[u8],
        mask: bool,
    ) -> Result<(), Error> {
        if payload.len() > MAX_WEBSOCKET_BODY {
            return Err(Error::Startup("WebSocket frame is too large".to_owned()));
        }
        if opcode >= 0x8 && (payload.len() > 125 || opcode & 0x40 != 0) {
            return Err(Error::Startup(
                "WebSocket control frame is invalid".to_owned(),
            ));
        }
        let mut header = vec![0x80 | opcode];
        let length = payload.len();
        let mask_bit = if mask { 0x80 } else { 0 };
        if length <= 125 {
            header.push(
                mask_bit
                    | u8::try_from(length).map_err(|_| {
                        Error::Startup("WebSocket frame length is invalid".to_owned())
                    })?,
            );
        } else if let Ok(length) = u16::try_from(length) {
            header.push(mask_bit | 0x7e);
            header.extend_from_slice(&length.to_be_bytes());
        } else {
            header.push(mask_bit | 127);
            header.extend_from_slice(&(length as u64).to_be_bytes());
        }
        self.stream.write_all(&header)?;
        if mask {
            let mut key = [0; 4];
            getrandom::fill(&mut key)
                .map_err(|error| Error::Startup(format!("WebSocket mask failed: {error}")))?;
            self.stream.write_all(&key)?;
            let mut masked = payload.to_vec();
            for (index, byte) in masked.iter_mut().enumerate() {
                *byte ^= key[index % key.len()];
            }
            self.stream.write_all(&masked)?;
        } else {
            self.stream.write_all(payload)?;
        }
        self.stream.flush()?;
        Ok(())
    }
}

fn parse_websocket_frame(
    buffer: &mut Vec<u8>,
    expect_mask: bool,
) -> Result<Option<WebSocketFrame>, Error> {
    let Some(header) = parse_websocket_header(buffer, expect_mask)? else {
        return Ok(None);
    };
    if buffer.len() < header.frame_length {
        return Ok(None);
    }
    let key = header.mask_offset.map(|offset| {
        [
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
        ]
    });
    let mut payload = buffer[header.payload_offset..header.frame_length].to_vec();
    buffer.drain(..header.frame_length);
    if let Some(key) = key {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[index % key.len()];
        }
    }
    Ok(Some(WebSocketFrame {
        final_frame: header.final_frame,
        opcode: header.opcode,
        payload,
    }))
}

struct WebSocketHeader {
    final_frame: bool,
    opcode: u8,
    payload_offset: usize,
    frame_length: usize,
    mask_offset: Option<usize>,
}

fn parse_websocket_header(
    buffer: &[u8],
    expect_mask: bool,
) -> Result<Option<WebSocketHeader>, Error> {
    if buffer.len() < 2 {
        return Ok(None);
    }
    let first = buffer[0];
    let second = buffer[1];
    if first & 0x70 != 0 {
        return Err(Error::Startup(
            "WebSocket reserved bits are unsupported".to_owned(),
        ));
    }
    let masked = second & 0x80 != 0;
    if masked != expect_mask {
        let message = if expect_mask {
            "client WebSocket frames must be masked"
        } else {
            "server WebSocket frames must not be masked"
        };
        return Err(Error::Startup(message.to_owned()));
    }
    let mut offset = 2;
    let length = match second & 0x7f {
        length @ 0..=125 => usize::from(length),
        126 => {
            if buffer.len() < offset + 2 {
                return Ok(None);
            }
            let length = u16::from_be_bytes([buffer[offset], buffer[offset + 1]]);
            offset += 2;
            usize::from(length)
        }
        127 => {
            if buffer.len() < offset + 8 {
                return Ok(None);
            }
            let length = u64::from_be_bytes(
                buffer[offset..offset + 8]
                    .try_into()
                    .map_err(|_| Error::Startup("WebSocket frame length is invalid".to_owned()))?,
            );
            offset += 8;
            usize::try_from(length)
                .map_err(|_| Error::Startup("WebSocket frame is too large".to_owned()))?
        }
        _ => unreachable!(),
    };
    if length > MAX_WEBSOCKET_BODY {
        return Err(Error::Startup("WebSocket frame is too large".to_owned()));
    }
    let opcode = first & 0x0f;
    let final_frame = first & 0x80 != 0;
    if opcode >= 0x8 && (!final_frame || length > 125) {
        return Err(Error::Startup(
            "WebSocket control frame is invalid".to_owned(),
        ));
    }
    let mask_offset = masked.then_some(offset);
    if masked {
        offset += 4;
    }
    let frame_length = offset
        .checked_add(length)
        .ok_or_else(|| Error::Startup("WebSocket frame is too large".to_owned()))?;
    Ok(Some(WebSocketHeader {
        final_frame,
        opcode,
        payload_offset: offset,
        frame_length,
        mask_offset,
    }))
}

pub(super) fn queue_websocket_message(
    bridge: &WebSocketBridge,
    fragmented: &mut Option<(u8, Vec<u8>)>,
    final_frame: bool,
    opcode: u8,
    payload: Vec<u8>,
) -> Result<(), Error> {
    let (message_opcode, payload) = match opcode {
        0x0 => {
            let Some((initial_opcode, mut message)) = fragmented.take() else {
                return Err(Error::Startup(
                    "WebSocket continuation has no initial frame".to_owned(),
                ));
            };
            if message.len().saturating_add(payload.len()) > MAX_WEBSOCKET_BODY {
                return Err(Error::Startup("WebSocket message is too large".to_owned()));
            }
            message.extend_from_slice(&payload);
            (initial_opcode, message)
        }
        0x1 | 0x2 => {
            if fragmented.is_some() {
                return Err(Error::Startup(
                    "WebSocket message starts before the previous message ended".to_owned(),
                ));
            }
            (opcode, payload)
        }
        _ => return Err(Error::Startup("invalid WebSocket opcode".to_owned())),
    };
    if final_frame {
        bridge
            .incoming
            .send(WebSocketInbound::Message {
                binary: message_opcode == 0x2,
                payload,
            })
            .map_err(|_| Error::Startup("WebSocket worker closed".to_owned()))?;
    } else {
        *fragmented = Some((message_opcode, payload));
    }
    Ok(())
}

fn flush_websocket_outbound<S: Read + Write>(
    codec: &mut WebSocketCodec<S>,
    outgoing: &Receiver<WebSocketOutbound>,
) -> Result<bool, Error> {
    loop {
        let frame = match outgoing.try_recv() {
            Ok(frame) => frame,
            Err(TryRecvError::Empty) => return Ok(false),
            Err(TryRecvError::Disconnected) => {
                return Err(Error::Startup("WebSocket worker closed".to_owned()));
            }
        };
        match frame {
            WebSocketOutbound::Message { binary, payload } => {
                codec.write_frame(if binary { 0x2 } else { 0x1 }, &payload, false)?;
            }
            WebSocketOutbound::Close { code, reason } => {
                let payload = websocket_close_payload(code, &reason)?;
                codec.write_frame(0x8, &payload, false)?;
                return Ok(true);
            }
            WebSocketOutbound::Ready => {}
        }
    }
}

fn wait_for_websocket_ready<S: Read + Write>(
    codec: &mut WebSocketCodec<S>,
    outgoing: &Receiver<WebSocketOutbound>,
) -> Result<bool, Error> {
    loop {
        match outgoing.recv_timeout(Duration::from_secs(30)) {
            Ok(WebSocketOutbound::Message { binary, payload }) => {
                codec.write_frame(if binary { 0x2 } else { 0x1 }, &payload, false)?;
            }
            Ok(WebSocketOutbound::Close { code, reason }) => {
                let payload = websocket_close_payload(code, &reason)?;
                codec.write_frame(0x8, &payload, false)?;
                return Ok(true);
            }
            Ok(WebSocketOutbound::Ready) => return Ok(false),
            Err(_) => {
                return Err(Error::Startup(
                    "WebSocket worker did not become ready".to_owned(),
                ));
            }
        }
    }
}

pub(super) fn websocket_close(payload: &[u8]) -> Result<(u16, String), Error> {
    if payload.is_empty() {
        return Ok((1000, String::new()));
    }
    if payload.len() == 1 {
        return Err(Error::Startup(
            "WebSocket close payload is invalid".to_owned(),
        ));
    }
    let code = u16::from_be_bytes([payload[0], payload[1]]);
    if !valid_websocket_close_code(code) {
        return Err(Error::Startup("WebSocket close code is invalid".to_owned()));
    }
    let reason = std::str::from_utf8(&payload[2..])
        .map_err(|_| Error::Startup("WebSocket close reason is invalid".to_owned()))?
        .to_owned();
    Ok((code, reason))
}

fn valid_websocket_close_code(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

pub(super) fn websocket_close_payload(code: u16, reason: &str) -> Result<Vec<u8>, Error> {
    if !valid_websocket_close_code(code) {
        return Err(Error::Startup("WebSocket close code is invalid".to_owned()));
    }
    if reason.len() > 123 {
        return Err(Error::Startup(
            "WebSocket close reason is too long".to_owned(),
        ));
    }
    let mut payload = Vec::with_capacity(2 + reason.len());
    payload.extend_from_slice(&code.to_be_bytes());
    payload.extend_from_slice(reason.as_bytes());
    Ok(payload)
}

pub(super) fn websocket_accept(key: &str) -> String {
    let digest = sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes());
    STANDARD.encode(digest)
}

pub(super) fn write_plain_response(
    stream: &mut TcpStream,
    response: HttpResponse,
) -> Result<(), Error> {
    write_response_inner(stream, response, None, None, None)
}

pub(super) fn write_response(
    stream: &mut SslStream<TcpStream>,
    response: HttpResponse,
    method: &str,
    cancelled: &AtomicBool,
) -> Result<(), Error> {
    let peer = stream.get_ref().try_clone()?;
    peer.set_read_timeout(Some(Duration::from_millis(1)))?;
    write_response_inner(stream, response, Some(method), Some(cancelled), Some(&peer))
}

fn write_response_inner(
    mut stream: impl Write,
    mut response: HttpResponse,
    method: Option<&str>,
    cancelled: Option<&AtomicBool>,
    peer: Option<&TcpStream>,
) -> Result<(), Error> {
    let head = method.is_some_and(|method| method.eq_ignore_ascii_case("HEAD"));
    let no_body =
        head || (100..200).contains(&response.status) || matches!(response.status, 204 | 205 | 304);
    if !response.headers.contains_key("connection") {
        response
            .headers
            .try_insert("connection", HeaderValue::from_static("close"))
            .map_err(io::Error::other)?;
    }
    if !head {
        response.headers.remove("transfer-encoding");
        if (100..200).contains(&response.status) || matches!(response.status, 204 | 304) {
            response.headers.remove("content-length");
        } else {
            let length = match &response.body {
                HttpBody::Buffered(body) => Some(body.len()),
                HttpBody::Stream(_) => None,
            };
            if let Some(length) = length {
                response
                    .headers
                    .try_insert("content-length", HeaderValue::from(length))
                    .map_err(io::Error::other)?;
            } else {
                response.headers.remove("content-length");
                response
                    .headers
                    .try_insert("transfer-encoding", HeaderValue::from_static("chunked"))
                    .map_err(io::Error::other)?;
            }
        }
    } else if response.status == 205
        && !response.headers.contains_key("content-length")
        && !response.headers.contains_key("transfer-encoding")
    {
        response
            .headers
            .try_insert("content-length", HeaderValue::from_static("0"))
            .map_err(io::Error::other)?;
    }
    write_response_headers(
        &mut stream,
        response.status,
        &response.status_text,
        &response.headers,
    )?;
    if no_body {
        return Ok(());
    }
    match response.body {
        HttpBody::Buffered(body) => {
            stream.write_all(&body)?;
            stream.flush()?;
            Ok(())
        }
        HttpBody::Stream(body) => write_stream_body(stream, &body, cancelled, peer),
    }
}

fn write_stream_body(
    mut stream: impl Write,
    body: &BodyStream,
    cancelled: Option<&AtomicBool>,
    peer: Option<&TcpStream>,
) -> Result<(), Error> {
    loop {
        let chunk = match body.receiver.recv_timeout(RESPONSE_STREAM_POLL) {
            Ok(chunk) => chunk,
            Err(flume::RecvTimeoutError::Timeout) => {
                if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
                    || body.cancelled.is_cancelled()
                {
                    break;
                }
                if peer_closed(peer) {
                    body.cancelled.cancel();
                    return Ok(());
                }
                continue;
            }
            Err(flume::RecvTimeoutError::Disconnected) => break,
        };
        let chunk =
            chunk.map_err(|error| Error::Startup(format!("response stream failed: {error}")))?;
        if !chunk.is_empty() {
            write!(stream, "{:X}\r\n", chunk.len())?;
            stream.write_all(&chunk)?;
            stream.write_all(b"\r\n")?;
            stream.flush()?;
        }
    }
    if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire))
        || body.cancelled.is_cancelled()
    {
        return Ok(());
    }
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()?;
    Ok(())
}

fn peer_closed(stream: Option<&TcpStream>) -> bool {
    let Some(stream) = stream else {
        return false;
    };
    let mut byte = [0; 1];
    match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
            ) =>
        {
            false
        }
        Err(_) => true,
    }
}

fn write_response_headers(
    stream: &mut impl Write,
    status: u16,
    reason: &str,
    headers: &HeaderMap,
) -> Result<(), Error> {
    hyper::ext::ReasonPhrase::try_from(reason.as_bytes()).map_err(io::Error::other)?;
    write!(stream, "HTTP/1.1 {status} {reason}\r\n")?;
    for (name, value) in headers {
        write!(stream, "{name}: ")?;
        stream.write_all(value.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.flush()?;
    Ok(())
}

fn tls_error(error: &openssl::error::ErrorStack) -> Error {
    Error::Tls(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{HttpBody, HttpResponse, read_request, response_stream, write_response_inner};
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    use std::io::{self, Cursor};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn rejects_excess_response_headers_without_panicking() -> Result<(), Box<dyn std::error::Error>>
    {
        for scenario in 0..3 {
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
            let mut output = Vec::new();
            assert!(write_response_inner(&mut output, response, None, None, None).is_err());
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
        let request = read_request(&mut input, "app.test")?;
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
        write_response_inner(&mut output, response, None, None, None)?;
        assert!(output.starts_with(b"HTTP/1.1 201 Created Here\r\n"));

        let mut response = HttpResponse::text(200, "ok");
        response.status_text = "OK\r\nInjected: true".to_owned();
        let mut output = Vec::new();
        assert!(write_response_inner(&mut output, response, None, None, None).is_err());
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
        write_response_inner(&mut output, response, None, None, None)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("set-cookie: first=1\r\n"));
        assert!(output.contains("set-cookie: second=2\r\n"));
        assert!(output.starts_with("HTTP/1.1 201 Created\r\n"));
        Ok(())
    }

    #[test]
    fn computes_response_framing_and_preserves_explicit_head_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = HttpResponse::text(200, "ok");
        response
            .headers
            .insert("content-length", HeaderValue::from_static("99"));
        response
            .headers
            .insert("transfer-encoding", HeaderValue::from_static("identity"));
        let mut output = Vec::new();
        write_response_inner(&mut output, response, Some("GET"), None, None)?;
        let output = String::from_utf8(output)?;
        assert!(output.contains("content-length: 2\r\n"));
        assert!(!output.contains("transfer-encoding:"));

        for explicit in [false, true] {
            let (_, cancelled, body) = response_stream();
            let mut response = HttpResponse::text(200, "");
            response.body = body;
            if explicit {
                response
                    .headers
                    .insert("content-length", HeaderValue::from_static("99"));
            }
            let mut output = Vec::new();
            write_response_inner(&mut output, response, Some("HEAD"), None, None)?;
            let output = String::from_utf8(output)?;
            assert_eq!(output.contains("content-length: 99\r\n"), explicit);
            assert!(!output.contains("transfer-encoding:"));
            assert!(cancelled.is_cancelled());
        }
        for status in [204, 205, 304] {
            let mut response = HttpResponse::text(status, "");
            response
                .headers
                .insert("content-length", HeaderValue::from_static("99"));
            let mut output = Vec::new();
            write_response_inner(&mut output, response, Some("GET"), None, None)?;
            let output = String::from_utf8(output)?;
            assert_eq!(output.contains("content-length: 0\r\n"), status == 205);
            assert!(!output.contains("content-length: 99"));
            assert!(!output.contains("transfer-encoding:"));
        }
        Ok(())
    }

    #[test]
    fn writes_buffered_responses_with_a_content_length() -> Result<(), Box<dyn std::error::Error>> {
        let response = HttpResponse::buffered(200, HeaderMap::new(), b"ok".to_vec());
        let mut output = Vec::new();

        write_response_inner(&mut output, response, None, None, None)?;

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

        write_response_inner(&mut output, response, None, None, None)?;

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

        write_response_inner(&mut output, response, Some("HEAD"), None, None)?;

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

        write_response_inner(&mut output, response, None, Some(&cancelled), None)?;

        assert!(String::from_utf8(output)?.contains("HTTP/1.1 200"));
        Ok(())
    }

    #[test]
    fn stops_an_idle_stream_when_the_client_disconnects() -> Result<(), Box<dyn std::error::Error>>
    {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let client = TcpStream::connect(address)?;
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
            let result = write_response_inner(server, response, None, None, Some(&peer));
            let _ = done_sender.send(result);
        });

        let mut client = client;
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0; 1];
            std::io::Read::read_exact(&mut client, &mut byte)?;
            headers.push(byte[0]);
        }
        drop(client);
        match done_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result?,
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
            Err(error) if error.to_string() == "QuickJS startup failed: HTTP body exceeds the limit"
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
        let error = match write_response_inner(io::sink(), response, None, None, None) {
            Ok(()) => return Err("stream should fail".into()),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "QuickJS startup failed: response stream failed: broken"
        );
        Ok(())
    }
}
