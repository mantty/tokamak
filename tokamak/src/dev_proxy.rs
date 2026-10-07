//! Development requests forwarded to the host framework server.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flume::Sender;
use futures_util::TryStreamExt;
use http_body_util::{BodyDataStream, BodyExt};
use hyper::body::{Body as _, Incoming};
use hyper::header::{HeaderMap, HeaderValue};
use hyper::upgrade::Upgraded;
use hyper::{Response, StatusCode};
use hyper_util::client::legacy;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::gateway::{
    Handler, HandlerError, Job, JobResponse, WebSocketInbound, WebSocketJob, WebSocketOutbound,
};
use crate::network::http::{Client, full, reason_phrase};
use crate::transport::{
    BodyChunk, HttpBody, HttpRequest, HttpResponse, WebSocketFrame, encode_websocket_frame,
    invalid_data, parse_websocket_frame, queue_websocket_message, response_stream,
    websocket_accept, websocket_close, websocket_close_payload,
};

const HTTP_RETRY_DELAY: Duration = Duration::from_millis(100);
const HTTP_RETRY_TIMEOUT: Duration = Duration::from_secs(10);

/// Headers that describe only one hop of a proxied exchange.
const HOP_BY_HOP: [&str; 7] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];
/// Request headers the proxy sets itself.
const PROXY_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-tokamak-session",
];

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
    client: Client,
    endpoint: Url,
    session_token: HeaderValue,
}

/// A host response body of unknown length, forwarded as it arrives.
struct HostBody {
    upstream: BodyDataStream<Incoming>,
    sender: Sender<BodyChunk>,
    cancelled: CancellationToken,
}

impl DevProxy {
    pub(crate) fn new(config: &DevProxyConfig) -> io::Result<Arc<Self>> {
        let session_token = config.session_token.trim();
        if session_token.is_empty() || session_token.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "development session token must be non-empty and contain no control characters",
            ));
        }
        Ok(Arc::new(Self {
            client: crate::network::http::client()?,
            endpoint: endpoint(&config.endpoint)?,
            session_token: HeaderValue::from_bytes(session_token.as_bytes())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
        }))
    }

    async fn forward_http(
        &self,
        request: &HttpRequest,
        response: &Sender<JobResponse>,
    ) -> Result<(), HandlerError> {
        let deadline = Instant::now() + HTTP_RETRY_TIMEOUT;
        let (head, body) = loop {
            match self.fetch(request).await {
                Ok(fetched) => break fetched,
                Err(error) if can_retry(&request.method, &error) && Instant::now() < deadline => {
                    tokio::time::sleep(HTTP_RETRY_DELAY).await;
                }
                Err(error) => return Err(error),
            }
        };
        // A caller that stopped waiting gets no response.
        if response.send(JobResponse::Http(head)).is_ok()
            && let Some(body) = body
        {
            body.forward().await;
        }
        Ok(())
    }

    /// The host server's response to `request`. A body of known length
    /// arrives whole; any other body streams.
    async fn fetch(
        &self,
        request: &HttpRequest,
    ) -> Result<(HttpResponse, Option<HostBody>), HandlerError> {
        let upstream = self.send(request, false).await?;
        let mut headers = upstream.headers().clone();
        for name in HOP_BY_HOP {
            headers.remove(name);
        }
        let mut head = HttpResponse::buffered(upstream.status().as_u16(), headers, Vec::new());
        head.status_text = reason_phrase(&upstream).into_owned();
        if upstream.body().size_hint().exact().is_some() {
            let body = upstream.into_body().collect().await?;
            head.body = HttpBody::Buffered(body.to_bytes().into());
            return Ok((head, None));
        }
        let (sender, cancelled, body) = response_stream();
        head.body = body;
        let body = HostBody {
            upstream: upstream.into_body().into_data_stream(),
            sender,
            cancelled,
        };
        Ok((head, Some(body)))
    }

    async fn send(
        &self,
        request: &HttpRequest,
        websocket: bool,
    ) -> Result<Response<Incoming>, HandlerError> {
        let mut upstream = hyper::Request::builder()
            .method(request.method.as_bytes())
            .uri(self.url(&request.target).as_str())
            .body(full(request.body.clone().unwrap_or_default()))?;
        *upstream.headers_mut() = self.forwarded_headers(request, websocket)?;
        Ok(self.client.request(upstream).await?)
    }

    /// The host server's URL for the request target `target`.
    fn url(&self, target: &str) -> Url {
        let (path, query) = target
            .split_once('?')
            .map_or((target, None), |(path, query)| (path, Some(query)));
        let mut url = self.endpoint.clone();
        url.set_path(path);
        url.set_query(query);
        url
    }

    /// The headers of `request` the host server receives, with the host,
    /// forwarding and session headers the proxy adds.
    fn forwarded_headers(
        &self,
        request: &HttpRequest,
        websocket: bool,
    ) -> Result<HeaderMap, HandlerError> {
        let mut headers = request.headers.clone();
        for name in HOP_BY_HOP.into_iter().chain(PROXY_HEADERS) {
            headers.remove(name);
        }
        if websocket {
            headers.remove("sec-websocket-extensions");
            headers.try_insert("connection", HeaderValue::from_static("Upgrade"))?;
            headers.try_insert("upgrade", HeaderValue::from_static("websocket"))?;
        }
        let host = match request.headers.get("host") {
            Some(host) => host.clone(),
            None => HeaderValue::from_str(
                &self.endpoint[url::Position::BeforeHost..url::Position::AfterPort],
            )?,
        };
        headers.try_insert("host", host.clone())?;
        headers.try_insert("x-forwarded-host", host)?;
        headers.try_insert("x-forwarded-proto", HeaderValue::from_static("https"))?;
        headers.try_insert("x-tokamak-session", self.session_token.clone())?;
        Ok(headers)
    }

    async fn forward_websocket(
        &self,
        request: &HttpRequest,
        response: &Sender<JobResponse>,
        websocket: &WebSocketJob,
        stopped: &CancellationToken,
    ) -> Result<(), HandlerError> {
        let upstream = tokio::select! {
            upstream = self.open_websocket(request) => upstream?,
            () = stopped.cancelled() => return Ok(()),
        };
        let gateway_closed =
            || io::Error::new(io::ErrorKind::BrokenPipe, "WebSocket gateway closed");
        response
            .send(JobResponse::WebSocket)
            .map_err(|_| gateway_closed())?;
        websocket
            .outgoing
            .send(WebSocketOutbound::Ready)
            .map_err(|_| gateway_closed())?;
        let result = relay_websocket(upstream, websocket, stopped).await;
        if result.is_err() {
            let _ = websocket.outgoing.send(WebSocketOutbound::Close {
                code: 1011,
                reason: "development server connection failed".to_owned(),
            });
        }
        result.map_err(Into::into)
    }

    async fn open_websocket(&self, request: &HttpRequest) -> Result<Upgraded, HandlerError> {
        let key = request
            .headers
            .get("sec-websocket-key")
            .ok_or_else(|| invalid_data("WebSocket key is missing"))?
            .to_str()?;
        let upgrade = self.send(request, true).await?;
        if upgrade.status() != StatusCode::SWITCHING_PROTOCOLS {
            let message = format!("host WebSocket upgrade returned HTTP {}", upgrade.status());
            return Err(invalid_data(&message).into());
        }
        if upgrade
            .headers()
            .get("sec-websocket-accept")
            .is_none_or(|actual| *actual != websocket_accept(key))
        {
            let message = "host WebSocket upgrade returned an invalid accept key";
            return Err(invalid_data(message).into());
        }
        Ok(hyper::upgrade::on(upgrade).await?)
    }
}

impl HostBody {
    /// Forwards the body until it ends, fails or its reader goes away.
    async fn forward(mut self) {
        loop {
            let chunk = tokio::select! {
                chunk = self.upstream.try_next() => chunk,
                () = self.cancelled.cancelled() => return,
            };
            let chunk = match chunk {
                Ok(Some(chunk)) => Ok(chunk.to_vec()),
                Ok(None) => return,
                Err(error) => Err(error.to_string()),
            };
            let failed = chunk.is_err();
            if self.sender.send_async(chunk).await.is_err() || failed {
                return;
            }
        }
    }
}

impl Handler for DevProxy {
    fn handle(&self, job: Job, stopped: &CancellationToken) -> Result<(), HandlerError> {
        let Job {
            request,
            response,
            websocket,
        } = job;
        let tokio = tokio::runtime::Handle::try_current()?;
        let Some(websocket) = websocket else {
            return tokio.block_on(async {
                tokio::select! {
                    result = self.forward_http(&request, &response) => result,
                    () = stopped.cancelled() => Ok(()),
                }
            });
        };
        tokio.block_on(self.forward_websocket(&request, &response, &websocket, stopped))
    }
}

/// The base URL of the development server `value` names: an HTTP or HTTPS
/// origin.
fn endpoint(value: &str) -> io::Result<Url> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("development endpoint must be an http:// or https:// origin: {value}"),
        )
    };
    let url = Url::parse(value).map_err(|_| invalid())?;
    let origin = matches!(url.scheme(), "http" | "https")
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    origin.then_some(url).ok_or_else(invalid)
}

/// Whether a request with `method` that failed with `error` is sent again:
/// an idempotent request whose connection the host server dropped, as it
/// does while restarting.
fn can_retry(method: &str, error: &HandlerError) -> bool {
    let idempotent = method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD");
    let dropped = error.is::<hyper::Error>()
        || error
            .downcast_ref::<legacy::Error>()
            .is_some_and(|error| !error.is_connect());
    idempotent && dropped
}

/// Relays frames between the `WebView`'s WebSocket and the host server's
/// until either side closes or the runtime stops.
async fn relay_websocket(
    upstream: Upgraded,
    websocket: &WebSocketJob,
    stopped: &CancellationToken,
) -> io::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(TokioIo::new(upstream));
    let mut buffer = Vec::new();
    let mut fragmented = None;
    let mut bytes = [0; 8192];
    loop {
        while let Some(frame) = parse_websocket_frame(&mut buffer, false)? {
            if relay_host_frame(frame, websocket, &mut fragmented, &mut writer).await? {
                return Ok(());
            }
        }
        let count = tokio::select! {
            () = stopped.cancelled() => {
                return write_close(&mut writer, 1001, "tokamak runtime stopped").await;
            }
            inbound = websocket.incoming.recv_async() => {
                if relay_client_message(inbound.ok(), &mut writer).await? {
                    return Ok(());
                }
                continue;
            }
            count = reader.read(&mut bytes) => count?,
        };
        if count == 0 {
            let _ = websocket.outgoing.send(WebSocketOutbound::Close {
                code: 1000,
                reason: String::new(),
            });
            return Ok(());
        }
        buffer.extend_from_slice(&bytes[..count]);
    }
}

/// Passes a host frame to the `WebView`, reporting whether it closed the connection.
async fn relay_host_frame(
    frame: WebSocketFrame,
    websocket: &WebSocketJob,
    fragmented: &mut Option<(u8, Vec<u8>)>,
    writer: &mut (impl AsyncWrite + Unpin),
) -> io::Result<bool> {
    match frame.opcode {
        0x8 => {
            let (code, reason) = websocket_close(&frame.payload)?;
            let _ = websocket
                .outgoing
                .send(WebSocketOutbound::Close { code, reason });
            Ok(true)
        }
        0x9 => write_frame(writer, 0xA, &frame.payload)
            .await
            .map(|()| false),
        0xA => Ok(false),
        0x0..=0x2 => queue_websocket_message(
            &websocket.outgoing,
            fragmented,
            frame.final_frame,
            frame.opcode,
            frame.payload,
        )
        .map(|()| false),
        _ => Err(invalid_data("invalid host WebSocket opcode")),
    }
}

/// Passes a `WebView` message to the host, reporting whether the connection closed.
async fn relay_client_message(
    inbound: Option<WebSocketInbound>,
    writer: &mut (impl AsyncWrite + Unpin),
) -> io::Result<bool> {
    match inbound {
        Some(WebSocketInbound::Message { binary, payload }) => {
            let opcode = if binary { 0x2 } else { 0x1 };
            write_frame(writer, opcode, &payload).await.map(|()| false)
        }
        Some(WebSocketInbound::Close { code, reason }) => {
            write_close(writer, code, &reason).await.map(|()| true)
        }
        None => Ok(true),
    }
}

async fn write_close(
    writer: &mut (impl AsyncWrite + Unpin),
    code: u16,
    reason: &str,
) -> io::Result<()> {
    write_frame(writer, 0x8, &websocket_close_payload(code, reason)?).await
}

async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    opcode: u8,
    payload: &[u8],
) -> io::Result<()> {
    writer
        .write_all(&encode_websocket_frame(opcode, payload, true)?)
        .await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::{DevProxy, DevProxyConfig, HandlerError, can_retry, endpoint, full};

    #[test]
    fn parses_host_endpoints() -> Result<(), Box<dyn std::error::Error>> {
        let local = endpoint("http://localhost:5173")?;
        assert_eq!(local.host_str(), Some("localhost"));
        assert_eq!(local.port_or_known_default(), Some(5173));
        assert_eq!(local.scheme(), "http");

        let loopback = endpoint("https://[::1]")?;
        assert_eq!(loopback.host_str(), Some("[::1]"));
        assert_eq!(loopback.port_or_known_default(), Some(443));
        assert_eq!(loopback.scheme(), "https");
        Ok(())
    }

    #[test]
    fn rejects_invalid_endpoint_suffixes() {
        assert!(endpoint("http://[::1]unexpected").is_err());
        assert!(endpoint("http://localhost:5173/path").is_err());
        assert!(endpoint("ftp://localhost").is_err());
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

    #[tokio::test]
    async fn retries_head_after_an_upstream_disconnect() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let url = format!("http://{}/", listener.local_addr()?);
        let host = thread::spawn(move || listener.accept().map(drop));

        let request = hyper::Request::head(url).body(full(Vec::new()))?;
        let result = crate::network::http::client()?.request(request).await;
        host.join().map_err(|_| "host panicked")??;

        let error: HandlerError = result.err().ok_or("the request succeeded")?.into();
        assert!(can_retry("HEAD", &error));
        assert!(!can_retry("POST", &error));
        Ok(())
    }
}
