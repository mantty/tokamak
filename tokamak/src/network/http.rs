use std::borrow::Cow;
use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;
use tokamak_macros::host_function;

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
    Class, Constructor, Ctx, Exception, FromJs, Function, IntoJs, JsLifetime, Object, Promise,
    TypedArray, Value, class::Trace, function::This,
};
use tokamak_macros::JsType;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::{io::StreamReader, sync::CancellationToken};
use tower::{BoxError, timeout::Timeout, util::BoxCloneSyncService};
use url::Url;

use super::decoder::DecodedBody;
use crate::globals::ContentDecoder;
use crate::transport::append_header;
use crate::typed::{JsPromise, JsReadableStream, StringPairs};

type Body = BoxBody<Bytes, io::Error>;
pub(crate) type Client = legacy::Client<Connector, Body>;
type Connector = BoxCloneSyncService<Uri, MaybeHttpsStream<TokioIo<TcpStream>>, BoxError>;
type Reader = Pin<Box<dyn AsyncRead>>;
type ResponseBody = Rc<Mutex<Option<Reader>>>;
type UploadCompletion = LocalBoxFuture<'static, Result<(), String>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const TCP_KEEPALIVE: Duration = Duration::from_secs(15);
const ONE_TIME_BODY_REDIRECT: &str = "A request with a one-time-use body (it was initialized from a stream, not a buffer) encountered a redirect requiring the body to be retransmitted. To avoid this error in the future, construct this request from a buffer-like body initializer.";

#[host_function]
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

/// The HTTP client the Worker's fetches share.
#[derive(JsLifetime)]
struct HttpClient(Client);

/// Gives `ctx` the HTTP client its fetches share.
pub(crate) fn attach(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let client = client().map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    ctx.store_userdata(HttpClient(client))?;
    Ok(())
}

/// A request the Worker's `fetch` sends.
#[derive(FromJs, JsType)]
pub(crate) struct FetchRequest<'js> {
    url: String,
    method: String,
    headers: StringPairs,
    body: Option<FetchBody<'js>>,
    redirect: String,
}

/// A request body: its bytes, or a stream of them.
pub(crate) enum FetchBody<'js> {
    Bytes(TypedArray<'js, u8>),
    Stream(JsReadableStream<'js>),
}

impl<'js> FromJs<'js> for FetchBody<'js> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        match TypedArray::<u8>::from_value(value.clone()) {
            Ok(bytes) => Ok(Self::Bytes(bytes)),
            Err(_) => JsReadableStream::from_js(ctx, value).map(Self::Stream),
        }
    }
}

impl crate::typescript::JsType for FetchBody<'_> {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        format!("Uint8Array | {}", JsReadableStream::typescript(types))
    }
}

/// A fetch in progress: its controls, the end of its request body's upload,
/// and its response.
#[derive(IntoJs, JsType)]
pub(crate) struct FetchTask<'js> {
    fetch: Class<'js, HttpFetch>,
    upload: JsPromise<'js, ()>,
    response: JsPromise<'js, FetchedResponse>,
}

/// A fetched response's status line, headers and URL, whether it followed a
/// redirect or has no body, and its body's length when known.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct FetchedResponse {
    status: u16,
    status_text: String,
    headers: StringPairs,
    url: String,
    redirected: bool,
    bodyless: bool,
    length: Option<u64>,
}

/// Starts sending `request`.
#[host_function]
pub(crate) fn http_fetch<'js>(
    ctx: Ctx<'js>,
    request: FetchRequest<'js>,
) -> rquickjs::Result<FetchTask<'js>> {
    let client = ctx
        .userdata::<HttpClient>()
        .map(|client| client.0.clone())
        .ok_or_else(|| Exception::throw_internal(&ctx, "The HTTP client is not available"))?;
    // Cancelled when the fetch is aborted.
    let aborted = CancellationToken::new();
    // Cancelled when the fetch is aborted or its response body is cancelled.
    let cancelled = aborted.child_token();
    // Cancelled to stop sending the request body: when the fetch fails or redirects, or its
    // response ends.
    let upload_stopped = cancelled.child_token();
    let (body, upload) = request_body(&ctx, request.body, upload_stopped.clone(), aborted.clone())?;
    let outgoing = Request {
        url: Url::parse(&request.url).map_err(|error| failure(&ctx, error))?,
        method: Method::from_bytes(request.method.as_bytes())
            .map_err(|error| failure(&ctx, error))?,
        headers: request_headers(&ctx, request.headers)?,
        body,
        redirect: request.redirect,
    };
    let reader: ResponseBody = Rc::new(Mutex::new(None));
    let fetch = HttpFetch {
        aborted,
        cancelled: cancelled.clone(),
        upload_stopped: upload_stopped.clone(),
        reader: Rc::clone(&reader),
    };
    let upload_ctx = ctx.clone();
    let upload = JsPromise::wrap_future(&ctx, async move {
        upload.await.map_err(|error| failure(&upload_ctx, error))
    })?;
    let future_ctx = ctx.clone();
    let response = JsPromise::wrap_future(&ctx, async move {
        let response = tokio::select! {
            biased;
            () = cancelled.cancelled() => return Err(failure(&future_ctx, "Fetch was aborted")),
            response = send(&client, outgoing, &upload_stopped) => match response {
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
        fetched_response(&future_ctx, response, &reader)
    })?;
    Ok(FetchTask {
        fetch: Class::instance(ctx, fetch)?,
        upload,
        response,
    })
}

/// A fetch's controls and response body.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct HttpFetch {
    #[qjs(skip_trace)]
    aborted: CancellationToken,
    #[qjs(skip_trace)]
    cancelled: CancellationToken,
    #[qjs(skip_trace)]
    upload_stopped: CancellationToken,
    #[qjs(skip_trace)]
    reader: ResponseBody,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl HttpFetch {
    /// Aborts the fetch and drops its response body.
    pub fn abort(&self) {
        self.aborted.cancel();
        self.drop_body();
    }

    /// Cancels the response body and drops it.
    pub fn cancel(&self) {
        self.cancelled.cancel();
        self.drop_body();
    }

    /// The response body's next chunk, or undefined at its end.
    pub fn read<'js>(
        &self,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<JsPromise<'js, Option<TypedArray<'js, u8>>>> {
        let reader = Rc::clone(&self.reader);
        let cancelled = self.cancelled.clone();
        let upload_stopped = self.upload_stopped.clone();
        let promise_ctx = ctx.clone();
        JsPromise::wrap_future(&promise_ctx, async move {
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
        })
    }
}

impl HttpFetch {
    fn drop_body(&self) {
        if let Ok(mut body) = self.reader.try_lock() {
            body.take();
        }
    }
}

fn request_headers(ctx: &Ctx<'_>, pairs: StringPairs) -> rquickjs::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs.0 {
        append_header(&mut headers, &name, &value).map_err(|error| failure(ctx, error))?;
    }
    Ok(headers)
}

fn request_body<'js>(
    ctx: &Ctx<'js>,
    body: Option<FetchBody<'js>>,
    stopped: CancellationToken,
    aborted: CancellationToken,
) -> rquickjs::Result<(RequestBody, UploadCompletion)> {
    let ready = || futures_util::future::ready(Ok(())).boxed_local();
    let stream = match body {
        None => return Ok((RequestBody::Bytes(Vec::new()), ready())),
        Some(FetchBody::Bytes(bytes)) => {
            let bytes = bytes
                .as_bytes()
                .ok_or_else(|| failure(ctx, "Detached request body"))?;
            return Ok((RequestBody::Bytes(bytes.to_vec()), ready()));
        }
        Some(FetchBody::Stream(stream)) => stream.into_inner(),
    };
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
        let error = crate::errors::new_error(reader.ctx(), "Network connection lost.")?;
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

fn fetched_response(
    ctx: &Ctx<'_>,
    fetched: FetchResponse,
    reader: &ResponseBody,
) -> rquickjs::Result<FetchedResponse> {
    let response = fetched.response;
    let status = response.status().as_u16();
    let status_text = reason_phrase(&response).into_owned();
    let headers = super::headers::pairs(response.headers());
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
    .map_err(|error| failure(ctx, error))?;
    *reader.try_lock().map_err(|error| failure(ctx, error))? = Some(body);
    Ok(FetchedResponse {
        status,
        status_text,
        headers,
        url: fetched.url.into(),
        redirected: fetched.redirected,
        bodyless: fetched.bodyless,
        length,
    })
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
