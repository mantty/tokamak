use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use futures_util::{FutureExt, TryStreamExt, future::LocalBoxFuture};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::{
    Method, Response, StatusCode, Uri,
    body::{Body as _, Bytes, Frame, Incoming},
    header::{HeaderMap, HeaderValue},
};
use hyper_rustls::{HttpsConnector, MaybeHttpsStream};
use hyper_util::client::legacy::{self, connect::HttpConnector};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rquickjs::{
    Constructor, Ctx, Exception, Function, Object, Promise, TypedArray, Value,
    function::{Async, This},
};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::{io::StreamReader, sync::CancellationToken};
use tower::{BoxError, timeout::Timeout, util::BoxCloneSyncService};
use url::Url;

use super::decoder::DecodedBody;
use crate::globals::ContentDecoder;
use crate::transport::append_header;

type Body = BoxBody<Bytes, io::Error>;
pub(crate) type Client = legacy::Client<Connector, Body>;
type Connector = BoxCloneSyncService<Uri, MaybeHttpsStream<TokioIo<TcpStream>>, BoxError>;
type Reader = Pin<Box<dyn AsyncRead>>;
type ResponseBody = Rc<Mutex<Option<Reader>>>;
type UploadCompletion = LocalBoxFuture<'static, Result<(), String>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TCP_KEEPALIVE: Duration = Duration::from_secs(15);
const ONE_TIME_BODY_REDIRECT: &str = "A request with a one-time-use body (it was initialized from a stream, not a buffer) encountered a redirect requiring the body to be retransmitted. To avoid this error in the future, construct this request from a buffer-like body initializer.";

pub(crate) fn status_text(status: u16) -> &'static str {
    if status == 203 {
        return "Non-Authoritative Information";
    }
    // Workerd's default phrases predate 425 Too Early; unknown statuses use
    // their class name rather than an empty string.
    if status != 425
        && let Ok(code) = StatusCode::from_u16(status)
        && let Some(reason) = code.canonical_reason()
    {
        return reason;
    }
    match status / 100 {
        1 => "Informational",
        2 => "Successful",
        3 => "Redirection",
        4 => "Client Error",
        5 => "Server Error",
        _ => "",
    }
}

/// The reason phrase `response` arrived with.
pub(crate) fn reason_phrase(response: &Response<Incoming>) -> Cow<'_, str> {
    response
        .extensions()
        .get::<hyper::ext::ReasonPhrase>()
        .map_or_else(
            || Cow::Borrowed(response.status().canonical_reason().unwrap_or("")),
            |reason| String::from_utf8_lossy(reason.as_bytes()),
        )
}

pub(crate) fn client() -> io::Result<Client> {
    let tls = crate::tls::client_config().map_err(io::Error::other)?;
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_nodelay(true);
    http.set_connect_timeout(Some(CONNECT_TIMEOUT));
    http.set_keepalive(Some(TCP_KEEPALIVE));
    http.set_keepalive_interval(Some(TCP_KEEPALIVE));
    http.set_keepalive_retries(Some(3));
    #[cfg(any(target_os = "android", target_os = "linux"))]
    http.set_tcp_user_timeout(Some(Duration::from_secs(30)));
    // The timeout also covers name resolution and the TLS handshake.
    let connector = Timeout::new(HttpsConnector::from((http, tls)), CONNECT_TIMEOUT);
    Ok(legacy::Client::builder(TokioExecutor::new())
        .pool_timer(TokioTimer::new())
        .build(BoxCloneSyncService::new(connector)))
}

/// A request body of `bytes`.
pub(crate) fn full(bytes: Vec<u8>) -> Body {
    Full::from(bytes).map_err(|never| match never {}).boxed()
}

struct Request {
    url: Url,
    method: Method,
    headers: HeaderMap,
    body: RequestBody,
    redirect: String,
}

struct FetchResponse {
    response: Response<Incoming>,
    url: Url,
    redirected: bool,
    bodyless: bool,
}

enum RequestBody {
    Bytes(Vec<u8>),
    Stream(Option<Body>),
}

#[derive(Serialize)]
struct ResponseHeaders<'a>(#[serde(serialize_with = "super::headers::serialize")] &'a HeaderMap);

impl RequestBody {
    fn take(&mut self) -> io::Result<Body> {
        match self {
            Self::Bytes(bytes) => Ok(full(bytes.clone())),
            Self::Stream(body) => body.take().ok_or_else(|| {
                io::Error::other("Cannot replay a streaming request body after a redirect")
            }),
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
pub(crate) fn start<'js>(
    ctx: Ctx<'js>,
    client: &Client,
    url: String,
    method: String,
    headers: String,
    body: Value<'js>,
    redirect: String,
) -> rquickjs::Result<Object<'js>> {
    // Cancelled when the fetch is aborted.
    let aborted = CancellationToken::new();
    // Cancelled when the fetch is aborted or its response body is cancelled.
    let cancelled = aborted.child_token();
    // Cancelled to stop sending the request body: when the fetch fails or redirects, or its
    // response ends.
    let upload_stopped = cancelled.child_token();
    let (body, upload) = request_body(&ctx, body, upload_stopped.clone(), aborted.clone())?;
    let request = Request {
        url: Url::parse(&url).map_err(|error| failure(&ctx, error))?,
        method: Method::from_bytes(method.as_bytes()).map_err(|error| failure(&ctx, error))?,
        headers: request_headers(&ctx, &headers)?,
        body,
        redirect,
    };
    let task = Object::new(ctx.clone())?;
    let reader: ResponseBody = Rc::new(Mutex::new(None));
    task.set("abort", stop_function(&ctx, aborted, &reader)?)?;
    task.set("cancel", stop_function(&ctx, cancelled.clone(), &reader)?)?;
    let client = client.clone();
    let upload_ctx = ctx.clone();
    task.set(
        "upload",
        Promise::wrap_future(&ctx, async move {
            upload.await.map_err(|error| failure(&upload_ctx, error))
        })?,
    )?;
    let future_ctx = ctx.clone();
    let response = Promise::wrap_future(&ctx, async move {
        let response = tokio::select! {
            biased;
            () = cancelled.cancelled() => return Err(failure(&future_ctx, "Fetch was aborted")),
            response = send(&client, request, &upload_stopped) => match response {
                Ok(response) => response,
                Err(error) => {
                    upload_stopped.cancel();
                    return Err(failure(&future_ctx, error));
                }
            },
        };
        if response.bodyless {
            upload_stopped.cancel();
        }
        response_object(future_ctx, response, reader, cancelled, upload_stopped)
    })?;
    task.set("response", response)?;
    Ok(task)
}

/// A function that cancels `token` and drops the response body.
fn stop_function<'js>(
    ctx: &Ctx<'js>,
    token: CancellationToken,
    reader: &ResponseBody,
) -> rquickjs::Result<Function<'js>> {
    let reader = Rc::clone(reader);
    Function::new(ctx.clone(), move || {
        token.cancel();
        if let Ok(mut body) = reader.try_lock() {
            body.take();
        }
    })
}

fn request_headers(ctx: &Ctx<'_>, json: &str) -> rquickjs::Result<HeaderMap> {
    let pairs: Vec<(String, String)> =
        serde_json::from_str(json).map_err(|error| failure(ctx, error))?;
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        append_header(&mut headers, &name, &value).map_err(|error| failure(ctx, error))?;
    }
    Ok(headers)
}

fn request_body<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    stopped: CancellationToken,
    aborted: CancellationToken,
) -> rquickjs::Result<(RequestBody, UploadCompletion)> {
    let ready = || futures_util::future::ready(Ok(())).boxed_local();
    if value.is_null() || value.is_undefined() {
        return Ok((RequestBody::Bytes(Vec::new()), ready()));
    }
    if let Ok(bytes) = TypedArray::<u8>::from_value(value.clone()) {
        return Ok((
            RequestBody::Bytes(
                bytes
                    .as_bytes()
                    .ok_or_else(|| failure(ctx, "Detached request body"))?
                    .to_vec(),
            ),
            ready(),
        ));
    }
    let stream = Object::from_value(value)?;
    let get_reader: Function = stream.get("getReader")?;
    let reader: Object = get_reader.call((This(stream),))?;
    let (sender, receiver) = flume::bounded::<io::Result<Vec<u8>>>(1);
    let (finished, completion) = tokio::sync::oneshot::channel();
    ctx.spawn(async move {
        let ended = tokio::select! {
            biased;
            () = stopped.cancelled() => Ok(false),
            ended = pump_upload(&reader, &sender) => ended,
        };
        if let Err(error) = &ended {
            let _ = sender
                .send_async(Err(io::Error::other(error.to_string())))
                .await;
        }
        // workerd cancels a body the request stops reading.
        if stopped.is_cancelled() || matches!(ended, Ok(false)) {
            let _ = cancel_source(&reader, aborted.is_cancelled());
        }
        if let Ok(release) = reader.get::<_, Function>("releaseLock") {
            let _ = release.call::<_, ()>((This(reader),));
        }
        let _ = finished.send(ended.map(|_| ()).map_err(|error| error.to_string()));
    });
    let completion =
        async move { completion.await.map_err(|error| error.to_string())? }.boxed_local();
    let body = StreamBody::new(
        receiver
            .into_stream()
            .map_ok(|bytes| Frame::data(Bytes::from(bytes))),
    );
    Ok((RequestBody::Stream(Some(body.boxed())), completion))
}

/// Cancels the request body's source as workerd does: with no reason when the
/// fetch is aborted, and otherwise with a lost connection's error.
fn cancel_source(reader: &Object<'_>, aborted: bool) -> rquickjs::Result<()> {
    let cancel: Function = reader.get("cancel")?;
    let reason = if aborted {
        None
    } else {
        let error = Exception::from_message(reader.ctx().clone(), "Network connection lost.")?;
        error.set("retryable", true)?;
        Some(error)
    };
    cancel.call::<_, Promise>((This(reader.clone()), reason))?;
    Ok(())
}

/// Sends the body's chunks to the request, returning whether the body ended
/// before the request stopped reading it.
async fn pump_upload(
    reader: &Object<'_>,
    sender: &flume::Sender<io::Result<Vec<u8>>>,
) -> rquickjs::Result<bool> {
    let read: Function = reader.get("read")?;
    loop {
        let result: Promise = read.call((This(reader.clone()),))?;
        let result: Object = result.into_future().await?;
        if result.get::<_, bool>("done")? {
            return Ok(true);
        }
        let bytes: TypedArray<u8> = result.get("value")?;
        let bytes = bytes
            .as_bytes()
            .ok_or_else(|| failure(reader.ctx(), "Detached request body"))?
            .to_vec();
        if sender.send_async(Ok(bytes)).await.is_err() {
            return Ok(false);
        }
    }
}

async fn send(
    client: &Client,
    mut request: Request,
    upload_stopped: &CancellationToken,
) -> Result<FetchResponse, BoxError> {
    let mut visited = Vec::new();
    for count in 0..=20 {
        visited.push(request.url.to_string());
        let mut outgoing = hyper::Request::builder()
            .method(request.method.clone())
            .uri(request.url.as_str())
            .body(request.body.take()?)?;
        *outgoing.headers_mut() = request.headers.clone();
        let mut response = client.request(outgoing).await?;
        let status = response.status().as_u16();
        let location = response.headers().get("location");
        let bodyless = request.method == Method::HEAD || matches!(status, 204 | 205 | 304);
        let follow = matches!(status, 301 | 302 | 303 | 307 | 308) && request.redirect != "manual";
        let Some(location) = location.filter(|_| follow) else {
            return Ok(FetchResponse {
                response,
                url: request.url,
                redirected: count > 0,
                bodyless,
            });
        };
        if count == 20 {
            return Err(format!("Too many redirects.{}", visited.join(", ")).into());
        }
        request.url = request.url.join(location.to_str()?)?;
        while response.body_mut().frame().await.transpose()?.is_some() {}
        upload_stopped.cancel();
        if status != 303 && matches!(request.body, RequestBody::Stream(_)) {
            return Err(ONE_TIME_BODY_REDIRECT.into());
        }
        redirect_as_get(&mut request, status);
    }
    unreachable!("redirect limit is checked before following a redirect")
}

/// Makes `request` a bodyless GET where fetch requires it after a `status` redirect.
fn redirect_as_get(request: &mut Request, status: u16) {
    if (matches!(status, 301 | 302) && request.method == Method::POST)
        || (status == 303 && request.method != Method::GET && request.method != Method::HEAD)
    {
        request.method = Method::GET;
        request.body = RequestBody::Bytes(Vec::new());
        // workerd keeps the body's other Content-* headers.
        request.headers.remove("content-length");
    }
}

fn response_object<'js>(
    ctx: Ctx<'js>,
    response: FetchResponse,
    reader: ResponseBody,
    cancelled: CancellationToken,
    upload_stopped: CancellationToken,
) -> rquickjs::Result<Object<'js>> {
    let result = Object::new(ctx.clone())?;
    result.set("redirected", response.redirected)?;
    result.set("bodyless", response.bodyless)?;
    result.set("url", response.url.as_str())?;
    let response = response.response;
    result.set("status", response.status().as_u16())?;
    result.set("statusText", reason_phrase(&response).as_ref())?;
    result.set(
        "headers",
        serde_json::to_string(&ResponseHeaders(response.headers()))
            .map_err(|error| failure(&ctx, error))?,
    )?;
    let encoding = response.headers().get("content-encoding").cloned();
    let length = response.body().size_hint().exact();
    let body = StreamReader::new(
        response
            .into_body()
            .into_data_stream()
            .map_err(io::Error::other),
    );
    let (body, length) = decode_body(
        Box::pin(body),
        encoding.as_ref().map(HeaderValue::as_bytes),
        length,
    )
    .map_err(|error| failure(&ctx, error))?;
    if let Some(length) = length {
        result.set("length", length)?;
    }
    *reader.try_lock().map_err(|error| failure(&ctx, error))? = Some(body);
    result.set(
        "read",
        Function::new(
            ctx,
            Async(move |ctx: Ctx<'js>| {
                let reader = Rc::clone(&reader);
                let cancelled = cancelled.clone();
                let upload_stopped = upload_stopped.clone();
                async move {
                    let mut body = reader.lock().await;
                    let Some(stream) = body.as_mut() else {
                        return Ok(None);
                    };
                    let mut bytes = vec![0; 16384];
                    let count = tokio::select! {
                        biased;
                        () = cancelled.cancelled() => Err(io::Error::other("Fetch was aborted")),
                        count = stream.read(&mut bytes) => count,
                    };
                    match count {
                        Ok(0) => {
                            body.take();
                            upload_stopped.cancel();
                            Ok(None)
                        }
                        Ok(count) => {
                            bytes.truncate(count);
                            TypedArray::new(ctx, bytes).map(Some)
                        }
                        Err(error) => {
                            body.take();
                            upload_stopped.cancel();
                            Err(failure(&ctx, error))
                        }
                    }
                }
            }),
        )?,
    )?;
    Ok(result)
}

/// The body decoded from `encoding`, and its `length` when decoding leaves
/// the body unchanged.
fn decode_body(
    body: Reader,
    encoding: Option<&[u8]>,
    length: Option<u64>,
) -> io::Result<(Reader, Option<u64>)> {
    let decoder = encoding.map(ContentDecoder::new).transpose()?.flatten();
    Ok(match decoder {
        Some(decoder) => (Box::pin(DecodedBody::new(body, decoder)), None),
        None => (body, length),
    })
}

/// Throws a `TypeError` with `error` as its whole message, which the `QuickJS`
/// throw functions cut at 255 bytes.
fn failure(ctx: &Ctx<'_>, error: impl std::fmt::Display) -> rquickjs::Error {
    let error = ctx
        .globals()
        .get::<_, Constructor>("TypeError")
        .and_then(|constructor| constructor.construct::<_, Value>((error.to_string(),)));
    match error {
        Ok(error) => ctx.throw(error),
        Err(error) => error,
    }
}
