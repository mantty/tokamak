use std::io;
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use async_compression::tokio::bufread::GzipDecoder;
use futures_util::{
    FutureExt, TryStreamExt,
    future::{LocalBoxFuture, Shared},
};
use reqwest::{
    Client, Method, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use rquickjs::{
    Ctx, Exception, Function, Object, Promise, TypedArray, Value,
    function::{Async, This},
};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::Mutex;
use tokio_util::{io::StreamReader, sync::CancellationToken};

type Reader = Pin<Box<dyn AsyncRead + Send>>;
type ResponseBody = Rc<Mutex<Option<Reader>>>;
type UploadCompletion = Shared<LocalBoxFuture<'static, Result<(), String>>>;

pub(crate) fn status_text(status: u16) -> &'static str {
    if status == 203 {
        return "Non-Authoritative Information";
    }
    // Workerd's default phrases predate 425 Too Early; unknown statuses use
    // their class name rather than an empty string.
    if status != 425
        && let Ok(code) = reqwest::StatusCode::from_u16(status)
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

pub(crate) fn client() -> io::Result<Client> {
    let tls = crate::tls::client_config().map_err(io::Error::other)?;
    Client::builder()
        .use_preconfigured_tls((*tls).clone())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .no_proxy()
        .build()
        .map_err(io::Error::other)
}

struct Request {
    url: Url,
    method: Method,
    headers: HeaderMap,
    body: RequestBody,
    redirect: String,
}

struct FetchResponse {
    response: reqwest::Response,
    redirected: bool,
    bodyless: bool,
}

enum RequestBody {
    Bytes(Vec<u8>),
    Stream(Option<reqwest::Body>),
}

impl RequestBody {
    fn take(&mut self) -> io::Result<reqwest::Body> {
        match self {
            Self::Bytes(bytes) => Ok(bytes.clone().into()),
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
    let cancelled = CancellationToken::new();
    let (body, upload) = request_body(&ctx, body, cancelled.child_token())?;
    let request = Request {
        url: Url::parse(&url).map_err(|error| failure(&ctx, error))?,
        method: Method::from_bytes(method.as_bytes()).map_err(|error| failure(&ctx, error))?,
        headers: request_headers(&ctx, &headers)?,
        body,
        redirect,
    };
    let task = Object::new(ctx.clone())?;
    let reader: ResponseBody = Rc::new(Mutex::new(None));
    let cancel_token = cancelled.clone();
    let cancel_reader = Rc::clone(&reader);
    task.set(
        "cancel",
        Function::new(ctx.clone(), move || {
            cancel_token.cancel();
            if let Ok(mut body) = cancel_reader.try_lock() {
                body.take();
            }
        })?,
    )?;
    let client = client.clone();
    let upload_done = upload.clone();
    let upload_ctx = ctx.clone();
    task.set(
        "upload",
        Promise::wrap_future(&ctx, async move {
            upload_done
                .await
                .map_err(|error| failure(&upload_ctx, error))
        })?,
    )?;
    let future_ctx = ctx.clone();
    let response = Promise::wrap_future(&ctx, async move {
        let response = tokio::select! {
            biased;
            () = cancelled.cancelled() => return Err(failure(&future_ctx, "Fetch was aborted")),
            response = send(&client, request, &upload) => match response {
                Ok(response) => response,
                Err(error) => {
                    cancelled.cancel();
                    return Err(failure(&future_ctx, error));
                }
            },
        };
        response_object(future_ctx, response, reader, cancelled, upload)
    })?;
    task.set("response", response)?;
    Ok(task)
}

fn request_headers(ctx: &Ctx<'_>, json: &str) -> rquickjs::Result<HeaderMap> {
    let pairs: Vec<(String, String)> =
        serde_json::from_str(json).map_err(|error| failure(ctx, error))?;
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|error| failure(ctx, error))?;
        let value =
            HeaderValue::from_bytes(value.as_bytes()).map_err(|error| failure(ctx, error))?;
        headers
            .try_append(name, value)
            .map_err(|error| failure(ctx, error))?;
    }
    Ok(headers)
}

fn request_body<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    cancelled: CancellationToken,
) -> rquickjs::Result<(RequestBody, UploadCompletion)> {
    let ready = || futures_util::future::ready(Ok(())).boxed_local().shared();
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
        let result = tokio::select! {
            biased;
            () = cancelled.cancelled() => Ok(()),
            result = pump_upload(&reader, &sender) => result,
        };
        if let Err(error) = &result {
            let _ = sender
                .send_async(Err(io::Error::other(error.to_string())))
                .await;
        }
        if cancelled.is_cancelled()
            && let Ok(cancel) = reader.get::<_, Function>("cancel")
        {
            let _ = cancel.call::<_, Promise>((This(reader.clone()),));
        }
        if let Ok(release) = reader.get::<_, Function>("releaseLock") {
            let _ = release.call::<_, ()>((This(reader),));
        }
        let _ = finished.send(result.map_err(|error| error.to_string()));
    });
    let completion = async move { completion.await.map_err(|error| error.to_string())? }
        .boxed_local()
        .shared();
    Ok((
        RequestBody::Stream(Some(reqwest::Body::wrap_stream(receiver.into_stream()))),
        completion,
    ))
}

async fn pump_upload(
    reader: &Object<'_>,
    sender: &flume::Sender<io::Result<Vec<u8>>>,
) -> rquickjs::Result<()> {
    let read: Function = reader.get("read")?;
    loop {
        let result: Promise = read.call((This(reader.clone()),))?;
        let result: Object = result.into_future().await?;
        if result.get::<_, bool>("done")? {
            return Ok(());
        }
        let bytes: TypedArray<u8> = result.get("value")?;
        let bytes = bytes
            .as_bytes()
            .ok_or_else(|| failure(reader.ctx(), "Detached request body"))?
            .to_vec();
        if sender.send_async(Ok(bytes)).await.is_err() {
            return Ok(());
        }
    }
}

async fn send(
    client: &Client,
    mut request: Request,
    upload: &UploadCompletion,
) -> Result<FetchResponse, Box<dyn std::error::Error + Send + Sync>> {
    for count in 0..=20 {
        let mut response = client
            .request(request.method.clone(), request.url.clone())
            .headers(request.headers.clone())
            .body(request.body.take()?)
            .send()
            .await?;
        let status = response.status().as_u16();
        let location = response.headers().get("location");
        let bodyless = request.method == Method::HEAD || matches!(status, 204 | 205 | 304);
        if !matches!(status, 301 | 302 | 303 | 307 | 308)
            || location.is_none()
            || request.redirect == "manual"
        {
            return Ok(FetchResponse {
                response,
                redirected: count > 0,
                bodyless,
            });
        }
        if count == 20 {
            return Err("Redirect limit exceeded".into());
        }
        let Some(location) = location else {
            unreachable!("responses without a location return above")
        };
        request.url = request.url.join(location.to_str()?)?;
        while response.chunk().await?.is_some() {}
        upload.clone().await?;
        if (matches!(status, 301 | 302) && request.method == Method::POST)
            || (status == 303 && request.method != Method::GET && request.method != Method::HEAD)
        {
            request.method = Method::GET;
            request.body = RequestBody::Bytes(Vec::new());
            for name in [
                "content-encoding",
                "content-language",
                "content-location",
                "content-type",
                "content-length",
            ] {
                request.headers.remove(name);
            }
        }
    }
    unreachable!("redirect limit is checked before following a redirect")
}

fn response_object<'js>(
    ctx: Ctx<'js>,
    response: FetchResponse,
    reader: ResponseBody,
    cancelled: CancellationToken,
    upload: UploadCompletion,
) -> rquickjs::Result<Object<'js>> {
    let result = Object::new(ctx.clone())?;
    result.set("redirected", response.redirected)?;
    result.set("bodyless", response.bodyless)?;
    let response = response.response;
    result.set("url", response.url().as_str())?;
    result.set("status", response.status().as_u16())?;
    result.set(
        "statusText",
        response
            .extensions()
            .get::<hyper::ext::ReasonPhrase>()
            .map_or_else(
                || std::borrow::Cow::Borrowed(response.status().canonical_reason().unwrap_or("")),
                |reason| String::from_utf8_lossy(reason.as_bytes()),
            )
            .as_ref(),
    )?;
    let headers: Vec<_> = response
        .headers()
        .iter()
        .map(|(name, value)| (name.as_str(), String::from_utf8_lossy(value.as_bytes())))
        .collect();
    result.set(
        "headers",
        serde_json::to_string(&headers).map_err(|error| failure(&ctx, error))?,
    )?;
    let encoding = response
        .headers()
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let body = StreamReader::new(response.bytes_stream().map_err(io::Error::other));
    *reader.try_lock().map_err(|error| failure(&ctx, error))? =
        Some(decode_body(Box::pin(body), &encoding));
    result.set(
        "read",
        Function::new(
            ctx,
            Async(move |ctx: Ctx<'js>| {
                let reader = Rc::clone(&reader);
                let cancelled = cancelled.clone();
                let upload = upload.clone();
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
                            upload.await.map_err(|error| failure(&ctx, error))?;
                            Ok(None)
                        }
                        Ok(count) => {
                            bytes.truncate(count);
                            TypedArray::new(ctx, bytes).map(Some)
                        }
                        Err(error) => {
                            body.take();
                            Err(failure(&ctx, error))
                        }
                    }
                }
            }),
        )?,
    )?;
    Ok(result)
}

fn decode_body(body: Reader, encoding: &str) -> Reader {
    match encoding {
        "gzip" => {
            let mut decoder = GzipDecoder::new(BufReader::new(body));
            decoder.multiple_members(true);
            Box::pin(decoder)
        }
        "br" => Box::pin(super::brotli::Decoder::new(body)),
        _ => body,
    }
}

fn failure(ctx: &Ctx<'_>, error: impl std::fmt::Display) -> rquickjs::Error {
    Exception::throw_type(ctx, &error.to_string())
}
