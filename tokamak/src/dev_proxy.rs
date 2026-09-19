//! Development requests forwarded to the host framework server.

use flume::TryRecvError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustls::{ClientConnection, StreamOwned};
use rustls_pki_types::ServerName;

use crate::gateway::{
    Execution, Handler, Job, JobResponse, WebSocketInbound, WebSocketJob, WebSocketOutbound,
};
use crate::quickjs::Error;
use crate::transport::{
    BodyChunk, HttpRequest, HttpResponse, MAX_HEADERS, WEBSOCKET_WRITE_TIMEOUT, WebSocketCodec,
    WebSocketRead, has_token, queue_websocket_message, read_header_block, response_stream,
    websocket_accept, websocket_close, websocket_close_payload,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_RETRY_DELAY: Duration = Duration::from_millis(100);
const HTTP_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
const STREAM_POLL: Duration = Duration::from_millis(100);
const WEBSOCKET_POLL: Duration = Duration::from_millis(20);

/// Host development-server connection settings.
#[derive(Clone, Debug)]
pub struct DevProxyConfig {
    /// HTTP or HTTPS endpoint exposed by the host development supervisor.
    pub endpoint: String,
    /// Credential identifying the current development session.
    pub session_token: String,
}

/// A gateway handler that forwards requests to the host development server.
pub(crate) struct DevProxy {
    endpoint: Endpoint,
    session_token: String,
}

struct HostResponse {
    response: HttpResponse,
    body: Option<HostBody>,
}

struct HostBody {
    upstream: UpstreamStream,
    framing: HostFraming,
    sender: flume::Sender<BodyChunk>,
    cancelled: tokio_util::sync::CancellationToken,
}

enum HostFraming {
    Chunked,
    CloseDelimited,
}

impl DevProxy {
    pub(crate) fn new(config: &DevProxyConfig) -> Result<Arc<Self>, Error> {
        let session_token = config.session_token.trim().to_owned();
        if session_token.is_empty() || session_token.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(Error::Startup(
                "development session token must be non-empty and contain no control characters"
                    .to_owned(),
            ));
        }
        Ok(Arc::new(Self {
            endpoint: Endpoint::parse(&config.endpoint)?,
            session_token,
        }))
    }

    fn connect(&self) -> Result<UpstreamStream, Error> {
        let addresses = (self.endpoint.host.as_str(), self.endpoint.port).to_socket_addrs()?;
        let stream = connect_any(addresses)?;
        stream.set_nodelay(true)?;
        if self.endpoint.tls {
            let config = crate::tls::client_config()
                .map_err(|error| Error::Startup(format!("host TLS setup failed: {error}")))?;
            let name = ServerName::try_from(self.endpoint.host.clone())
                .map_err(|error| Error::Startup(format!("host TLS setup failed: {error}")))?;
            let connection = ClientConnection::new(config, name)
                .map_err(|error| Error::Startup(format!("host TLS setup failed: {error}")))?;
            let mut stream = StreamOwned::new(connection, stream);
            while stream.conn.is_handshaking() {
                stream.conn.complete_io(&mut stream.sock).map_err(|error| {
                    Error::Startup(format!("host TLS connection failed: {error}"))
                })?;
            }
            Ok(UpstreamStream::Tls(Box::new(stream)))
        } else {
            Ok(UpstreamStream::Plain(stream))
        }
    }

    fn forward_http(
        &self,
        request: &HttpRequest,
        response: &flume::Sender<JobResponse>,
    ) -> Result<(), Error> {
        let deadline = Instant::now() + HTTP_RETRY_TIMEOUT;
        let host_response = loop {
            match self.request_http(request) {
                Ok(host_response) => break host_response,
                Err(error)
                    if can_retry_http(&request.method, &error) && Instant::now() < deadline =>
                {
                    thread::sleep(HTTP_RETRY_DELAY);
                }
                Err(error) => return Err(error),
            }
        };
        response
            .send(JobResponse::Http(host_response.response))
            .map_err(|_| Error::Startup("HTTP response receiver closed".to_owned()))?;
        if let Some(body) = host_response.body {
            pump_host_body(body);
        }
        Ok(())
    }

    fn request_http(&self, request: &HttpRequest) -> Result<HostResponse, Error> {
        let mut upstream = self.connect()?;
        upstream.set_timeouts(CONNECT_TIMEOUT, CONNECT_TIMEOUT)?;
        write_request(
            &mut upstream,
            &self.endpoint,
            &self.session_token,
            request,
            false,
        )?;
        let (mut response, framing) = read_response(&mut upstream, &request.method)?;
        let body = framing.map(|framing| {
            let (sender, cancelled, body) = response_stream();
            response.body = body;
            HostBody {
                upstream,
                framing,
                sender,
                cancelled,
            }
        });
        Ok(HostResponse { response, body })
    }

    fn forward_websocket(
        &self,
        request: &HttpRequest,
        response: &flume::Sender<JobResponse>,
        websocket: &WebSocketJob,
        execution: &Execution<'_>,
    ) -> Result<(), Error> {
        let mut codec = self.open_websocket(request)?;
        response
            .send(JobResponse::WebSocket)
            .map_err(|_| Error::Startup("WebSocket response receiver closed".to_owned()))?;
        websocket
            .outgoing
            .send(WebSocketOutbound::Ready)
            .map_err(|_| Error::Startup("WebSocket gateway closed".to_owned()))?;

        let result = proxy_websocket(&mut codec, websocket, execution);
        if result.is_err() {
            let _ = websocket.outgoing.send(WebSocketOutbound::Close {
                code: 1011,
                reason: "development server connection failed".to_owned(),
            });
        }
        result
    }

    fn open_websocket(
        &self,
        request: &HttpRequest,
    ) -> Result<WebSocketCodec<UpstreamStream>, Error> {
        let key = request
            .headers
            .get("sec-websocket-key")
            .ok_or_else(|| Error::Startup("WebSocket key is missing".to_owned()))?
            .to_str()
            .map_err(io::Error::other)?;
        let mut upstream = self.connect()?;
        upstream.set_timeouts(CONNECT_TIMEOUT, CONNECT_TIMEOUT)?;
        write_request(
            &mut upstream,
            &self.endpoint,
            &self.session_token,
            request,
            true,
        )?;
        let (upgrade, _) = read_response(&mut upstream, &request.method)?;
        if upgrade.status != 101 {
            return Err(Error::Startup(format!(
                "host WebSocket upgrade returned HTTP {}",
                upgrade.status
            )));
        }
        if upgrade
            .headers
            .get("sec-websocket-accept")
            .is_none_or(|actual| *actual != websocket_accept(key))
        {
            return Err(Error::Startup(
                "host WebSocket upgrade returned an invalid accept key".to_owned(),
            ));
        }
        upstream.set_timeouts(WEBSOCKET_POLL, WEBSOCKET_WRITE_TIMEOUT)?;
        Ok(WebSocketCodec::new(upstream))
    }
}

fn connect_any(addresses: impl Iterator<Item = SocketAddr>) -> io::Result<TcpStream> {
    let mut last_error = io::Error::new(
        io::ErrorKind::NotFound,
        "development endpoint did not resolve",
    );
    for address in addresses {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

fn can_retry_http(method: &str, error: &Error) -> bool {
    let Error::Io(error) = error else {
        return false;
    };
    (method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD"))
        && matches!(
            error.kind(),
            io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
        )
}

fn proxy_websocket(
    codec: &mut WebSocketCodec<UpstreamStream>,
    websocket: &WebSocketJob,
    execution: &Execution<'_>,
) -> Result<(), Error> {
    let mut fragmented: Option<(u8, Vec<u8>)> = None;
    loop {
        if !execution.is_running() {
            let close = websocket_close_payload(1001, "tokamak runtime stopped")?;
            codec.write_frame(0x8, &close, true)?;
            return Ok(());
        }
        loop {
            match websocket.incoming.try_recv() {
                Ok(WebSocketInbound::Message { binary, payload }) => {
                    codec.write_frame(if binary { 0x2 } else { 0x1 }, &payload, true)?;
                }
                Ok(WebSocketInbound::Close { code, reason }) => {
                    let close = websocket_close_payload(code, &reason)?;
                    codec.write_frame(0x8, &close, true)?;
                    return Ok(());
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        match codec.read_frame(false)? {
            WebSocketRead::Closed => {
                let _ = websocket.outgoing.send(WebSocketOutbound::Close {
                    code: 1000,
                    reason: String::new(),
                });
                return Ok(());
            }
            WebSocketRead::Pending => thread::sleep(WEBSOCKET_POLL),
            WebSocketRead::Frame(frame) => match frame.opcode {
                0x8 => {
                    let (code, reason) = websocket_close(&frame.payload)?;
                    let _ = websocket
                        .outgoing
                        .send(WebSocketOutbound::Close { code, reason });
                    return Ok(());
                }
                0x9 => codec.write_frame(0xA, &frame.payload, true)?,
                0xA => {}
                0x0..=0x2 => queue_websocket_message(
                    &websocket.outgoing,
                    &mut fragmented,
                    frame.final_frame,
                    frame.opcode,
                    frame.payload,
                )?,
                _ => return Err(Error::Startup("invalid host WebSocket opcode".to_owned())),
            },
        }
    }
}

impl Handler for DevProxy {
    fn handle(&self, job: Job, execution: &Execution<'_>) -> Result<(), Error> {
        if !execution.is_running() {
            return Ok(());
        }
        let Job {
            request,
            response,
            websocket,
        } = job;
        if let Some(websocket) = websocket {
            self.forward_websocket(&request, &response, &websocket, execution)
        } else {
            self.forward_http(&request, &response)
        }
    }
}

#[derive(Clone, Debug)]
struct Endpoint {
    host: String,
    port: u16,
    authority: String,
    tls: bool,
}

impl Endpoint {
    fn parse(value: &str) -> Result<Self, Error> {
        let (tls, authority) = if let Some(value) = value.strip_prefix("http://") {
            (false, value)
        } else if let Some(value) = value.strip_prefix("https://") {
            (true, value)
        } else {
            return Err(Error::Startup(
                "development endpoint must start with http:// or https://".to_owned(),
            ));
        };
        if authority.is_empty()
            || authority.contains(['/', '?', '#'])
            || authority.chars().any(char::is_whitespace)
        {
            return Err(Error::Startup(
                "development endpoint must contain only a host and port".to_owned(),
            ));
        }
        let default_port = if tls { 443 } else { 80 };
        let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
            let end = rest.find(']').ok_or_else(|| {
                Error::Startup("development endpoint has an invalid IPv6 host".to_owned())
            })?;
            let host = &rest[..end];
            let suffix = &rest[end + 1..];
            let port = if suffix.is_empty() {
                default_port
            } else {
                parse_port(suffix.strip_prefix(':').ok_or_else(|| {
                    Error::Startup("development endpoint has an invalid IPv6 host".to_owned())
                })?)?
            };
            (host.to_owned(), port)
        } else if let Some((host, port)) = authority.rsplit_once(':') {
            if host.is_empty() {
                return Err(Error::Startup(
                    "development endpoint host is empty".to_owned(),
                ));
            }
            if host.contains(':') {
                return Err(Error::Startup(
                    "development endpoint has an invalid IPv6 host".to_owned(),
                ));
            }
            (host.to_owned(), parse_port(port)?)
        } else {
            (authority.to_owned(), default_port)
        };
        if host.is_empty() || host.chars().any(char::is_control) {
            return Err(Error::Startup(
                "development endpoint host is invalid".to_owned(),
            ));
        }
        Ok(Self {
            authority: if host.contains(':') {
                format!("[{host}]:{port}")
            } else {
                format!("{host}:{port}")
            },
            host,
            port,
            tls,
        })
    }
}

fn parse_port(value: &str) -> Result<u16, Error> {
    value
        .parse()
        .map_err(|_| Error::Startup("development endpoint port is invalid".to_owned()))
}

enum UpstreamStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl UpstreamStream {
    fn socket(&mut self) -> &mut TcpStream {
        match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => stream.get_mut(),
        }
    }

    fn set_timeouts(&mut self, read: Duration, write: Duration) -> io::Result<()> {
        let socket = self.socket();
        socket.set_read_timeout(Some(read))?;
        socket.set_write_timeout(Some(write))
    }

    fn set_stream_timeout(&mut self) -> io::Result<()> {
        self.socket().set_read_timeout(Some(STREAM_POLL))
    }
}

impl Read for UpstreamStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(bytes),
            Self::Tls(stream) => stream.read(bytes),
        }
    }
}

impl Write for UpstreamStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(bytes),
            Self::Tls(stream) => stream.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn write_request(
    stream: &mut UpstreamStream,
    endpoint: &Endpoint,
    session_token: &str,
    request: &HttpRequest,
    websocket: bool,
) -> Result<(), Error> {
    let host = request
        .headers
        .get("host")
        .map(HeaderValue::to_str)
        .transpose()
        .map_err(io::Error::other)?
        .unwrap_or(endpoint.authority.as_str());
    write!(stream, "{} {} HTTP/1.1\r\n", request.method, request.target)?;
    for (name, value) in &request.headers {
        let name = name.as_str();
        if matches!(
            name,
            "host"
                | "content-length"
                | "x-forwarded-host"
                | "x-forwarded-proto"
                | "x-tokamak-session"
        ) || (websocket && name == "sec-websocket-extensions")
            || is_hop_by_hop(name)
        {
            continue;
        }
        write!(stream, "{name}: ")?;
        stream.write_all(value.as_bytes())?;
        stream.write_all(b"\r\n")?;
    }
    write!(stream, "Host: {host}\r\n")?;
    write!(stream, "X-Forwarded-Host: {host}\r\n")?;
    write!(stream, "X-Forwarded-Proto: https\r\n")?;
    write!(stream, "X-Tokamak-Session: {session_token}\r\n")?;
    if websocket {
        write!(stream, "Connection: Upgrade\r\nUpgrade: websocket\r\n")?;
    } else {
        let length = request.body.as_ref().map_or(0, Vec::len);
        write!(stream, "Connection: close\r\nContent-Length: {length}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    if !websocket && let Some(body) = &request.body {
        stream.write_all(body)?;
    }
    stream.flush()?;
    Ok(())
}

fn read_response(
    stream: &mut UpstreamStream,
    method: &str,
) -> Result<(HttpResponse, Option<HostFraming>), Error> {
    let headers = read_header_block(stream, "host response headers")?;
    let text = String::from_utf8_lossy(&headers);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut status_parts = status_line.splitn(3, ' ');
    let _version = status_parts.next();
    let status: u16 = status_parts
        .next()
        .ok_or_else(|| Error::Startup("host response status is missing".to_owned()))?
        .parse()
        .map_err(|_| Error::Startup("host response status is invalid".to_owned()))?;
    let status_text = status_parts.next().unwrap_or_default().to_owned();
    let mut response_headers = HeaderMap::new();
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
            content_length = Some(value.parse().map_err(|_| {
                Error::Startup("host response content length is invalid".to_owned())
            })?);
        }
        if name == "transfer-encoding" && has_token(value, "chunked") {
            chunked = true;
        }
        if !is_hop_by_hop(&name) {
            response_headers
                .try_append(
                    HeaderName::from_bytes(name.as_bytes()).map_err(io::Error::other)?,
                    HeaderValue::from_bytes(value.as_bytes()).map_err(io::Error::other)?,
                )
                .map_err(io::Error::other)?;
        }
    }
    if chunked {
        response_headers.remove("content-length");
    }
    let no_body = method.eq_ignore_ascii_case("HEAD")
        || (100..200).contains(&status)
        || matches!(status, 204 | 304);
    let (body, framing) = if no_body {
        (Vec::new(), None)
    } else if chunked {
        (Vec::new(), Some(HostFraming::Chunked))
    } else if let Some(length) = content_length {
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        (body, None)
    } else {
        (Vec::new(), Some(HostFraming::CloseDelimited))
    };
    let mut response = HttpResponse::buffered(status, response_headers, body);
    response.status_text = status_text;
    Ok((response, framing))
}

fn pump_host_body(mut body: HostBody) {
    let result = body
        .upstream
        .set_stream_timeout()
        .map_err(Error::from)
        .and_then(|()| match body.framing {
            HostFraming::Chunked => pump_chunked_body(&mut body),
            HostFraming::CloseDelimited => pump_close_delimited_body(&mut body),
        });
    if let Err(error) = result {
        let _ = body.sender.send(Err(error.to_string()));
    }
}

fn pump_close_delimited_body(body: &mut HostBody) -> Result<(), Error> {
    let mut buffer = [0; 16 * 1024];
    loop {
        let Some(count @ 1..) = read_host_chunk(body, &mut buffer)? else {
            return Ok(());
        };
        if !send_host_chunk(body, buffer[..count].to_vec()) {
            return Ok(());
        }
    }
}

fn pump_chunked_body(body: &mut HostBody) -> Result<(), Error> {
    let mut buffer = [0; 16 * 1024];
    loop {
        let Some(line) = read_upstream_line(body)? else {
            return Ok(());
        };
        let size = line.split(';').next().unwrap_or_default().trim();
        let mut remaining = usize::from_str_radix(size, 16)
            .map_err(|_| Error::Startup("host response chunk size is invalid".to_owned()))?;
        if remaining == 0 {
            read_chunked_trailers(body)?;
            return Ok(());
        }
        while remaining > 0 {
            let limit = remaining.min(buffer.len());
            let Some(count) = read_host_chunk(body, &mut buffer[..limit])? else {
                return Ok(());
            };
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "host response chunk ended early",
                )
                .into());
            }
            if !send_host_chunk(body, buffer[..count].to_vec()) {
                return Ok(());
            }
            remaining -= count;
        }
        let mut terminator = [0; 2];
        if !read_host_exact(body, &mut terminator)? {
            return Ok(());
        }
        if terminator != *b"\r\n" {
            return Err(Error::Startup(
                "host response chunk is not terminated".to_owned(),
            ));
        }
    }
}

fn send_host_chunk(body: &HostBody, chunk: Vec<u8>) -> bool {
    !chunk.is_empty() && body.sender.send(Ok(chunk)).is_ok()
}

fn read_host_chunk(body: &mut HostBody, buffer: &mut [u8]) -> Result<Option<usize>, Error> {
    loop {
        if body.cancelled.is_cancelled() {
            return Ok(None);
        }
        match body.upstream.read(buffer) {
            Ok(count) => return Ok(Some(count)),
            Err(error) if is_stream_timeout(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn read_host_exact(body: &mut HostBody, buffer: &mut [u8]) -> Result<bool, Error> {
    let mut offset = 0;
    while offset < buffer.len() {
        let Some(count) = read_host_chunk(body, &mut buffer[offset..])? else {
            return Ok(false);
        };
        if count == 0 {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "host response ended early").into(),
            );
        }
        offset += count;
    }
    Ok(true)
}

fn read_chunked_trailers(body: &mut HostBody) -> Result<(), Error> {
    let mut bytes = 0usize;
    loop {
        let Some(line) = read_upstream_line(body)? else {
            return Ok(());
        };
        bytes = bytes.saturating_add(line.len() + 2);
        if bytes > MAX_HEADERS {
            return Err(Error::Startup(
                "host response trailers exceed the limit".to_owned(),
            ));
        }
        if line.is_empty() {
            return Ok(());
        }
    }
}

fn read_upstream_line(body: &mut HostBody) -> Result<Option<String>, Error> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0; 1];
        if !read_host_exact(body, &mut byte)? {
            return Ok(None);
        }
        line.push(byte[0]);
        if line.len() > MAX_HEADERS {
            return Err(Error::Startup("host response line is too long".to_owned()));
        }
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line)
                .map(Some)
                .map_err(|_| Error::Startup("host response line is not UTF-8".to_owned()));
        }
    }
}

fn is_stream_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{DevProxy, DevProxyConfig, Endpoint, can_retry_http};
    use crate::quickjs::Error;

    #[test]
    fn parses_host_endpoints() -> Result<(), Box<dyn std::error::Error>> {
        let endpoint = Endpoint::parse("http://localhost:5173")?;
        assert_eq!(endpoint.host, "localhost");
        assert_eq!(endpoint.port, 5173);
        assert!(!endpoint.tls);

        let endpoint = Endpoint::parse("https://[::1]")?;
        assert_eq!(endpoint.host, "::1");
        assert_eq!(endpoint.port, 443);
        assert!(endpoint.tls);
        Ok(())
    }

    #[test]
    fn rejects_invalid_endpoint_suffixes() {
        assert!(Endpoint::parse("http://[::1]unexpected").is_err());
        assert!(Endpoint::parse("http://localhost:5173/path").is_err());
    }

    #[test]
    fn rejects_control_characters_in_session_tokens() {
        assert!(
            DevProxy::new(&DevProxyConfig {
                endpoint: "http://localhost:5173".to_owned(),
                session_token: "token\nforged-header: value".to_owned(),
            })
            .is_err()
        );
    }

    #[test]
    fn retries_head_after_an_upstream_disconnect() {
        let error = Error::Io(io::ErrorKind::UnexpectedEof.into());
        assert!(can_retry_http("HEAD", &error));
    }
}
