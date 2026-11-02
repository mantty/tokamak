//! The runtime bootstrap's functions, which `builtins/runtime.ts` exports as
//! `bootstrap`, and the values the dispatcher exchanges with them.

use std::sync::Arc;

use rquickjs::{
    ArrayBuffer, Class, Ctx, Exception, FromJs, IntoJs, JsLifetime, Object, TypedArray, Value,
    class::Trace,
};
use tokamak_macros::JsType;

use super::invocation::WaitUntilObserver;
use crate::assets::{AssetResponse, Assets};
use crate::gateway::WebSocketOutbound;
use crate::globals::plugins::{NativeHost, Platform};
use crate::services::{JsMessage, JsReply, ServiceHost, StreamCodec};
use crate::transport::HttpRequest;
use crate::typed::{JsFunction, JsPromise, StringPairs};

/// The runtime bootstrap's functions: the dispatcher's routes into the
/// runtime.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct Bootstrap<'js> {
    /// The Worker's `Request` for the host's request and its body.
    pub(crate) host_request:
        JsFunction<'js, (RequestDescriptor, Option<TypedArray<'js, u8>>), Object<'js>>,
    /// The parts of the Worker's response that the host sends.
    pub(crate) host_response: JsFunction<'js, (Value<'js>,), ResponseParts<'js>>,
    /// Delivers an event's encoded data to the `TokamakEvents` the Worker's
    /// entry module exports, resolving to its `DispatchedEvent`.
    pub(crate) dispatch_event:
        JsFunction<'js, (Object<'js>, String, TypedArray<'js, u8>), JsPromise<'js, Value<'js>>>,
    /// Binds the app's assets in the Worker's environment by name.
    pub(crate) install_assets: JsFunction<'js, (String, Class<'js, AssetFetcher>)>,
    /// Connects the Worker to the shell's plugins on the named platform.
    pub(crate) install_plugins: JsFunction<'js, (Class<'js, NativeHost>, Platform)>,
    /// Binds the named services in the Worker's environment.
    pub(crate) install_services: JsFunction<'js, (Vec<String>, Class<'js, ServiceHost>)>,
    /// Calls a method of a service entrypoint the Worker exports.
    pub(crate) invoke_service: JsFunction<
        'js,
        (Object<'js>, String, String, JsMessage<'js>),
        JsPromise<'js, JsReply<'js>>,
    >,
    /// Moves the streams in service messages.
    pub(crate) service_streams: StreamCodec<'js>,
    /// Tells the host as the Worker's `waitUntil` work starts and settles.
    pub(crate) track_wait_until: JsFunction<'js, (Class<'js, WaitUntilObserver>,)>,
}

/// The host's request, as `hostRequest` and `__tokamak_request` hold it.
#[derive(IntoJs, JsType)]
pub(crate) struct RequestDescriptor {
    method: String,
    url: String,
    headers: StringPairs,
}

impl RequestDescriptor {
    pub(super) fn new(request: &HttpRequest) -> Self {
        Self {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: crate::network::headers::pairs(&request.headers),
        }
    }
}

/// The status line, headers, encoding, body and native WebSocket end the host
/// sends for a Worker's response.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct ResponseParts<'js> {
    pub(super) status: u16,
    pub(super) status_text: String,
    pub(super) headers: StringPairs,
    pub(super) encode_body: String,
    pub(super) body: ResponseBody<'js>,
    pub(super) web_socket: Option<NativeSocket<'js>>,
}

/// A response's body: its bytes, or the runtime's reader of its stream.
pub(crate) enum ResponseBody<'js> {
    Buffered(TypedArray<'js, u8>),
    Stream(BodyReader<'js>),
}

impl<'js> FromJs<'js> for ResponseBody<'js> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        match TypedArray::<u8>::from_value(value.clone()) {
            Ok(bytes) => Ok(Self::Buffered(bytes)),
            Err(_) => BodyReader::from_js(ctx, value).map(Self::Stream),
        }
    }
}

impl crate::typescript::JsType for ResponseBody<'_> {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        format!("Uint8Array | {}", BodyReader::typescript(types))
    }
}

/// The runtime's reader of a response body stream.
#[derive(FromJs, JsType)]
pub(crate) struct BodyReader<'js> {
    /// Resolves to the stream's next chunk, or undefined at its end.
    pub(super) read: JsFunction<'js, (), JsPromise<'js, Option<TypedArray<'js, u8>>>>,
    pub(super) cancel: JsFunction<'js, (), JsPromise<'js, ()>>,
}

/// The host's end of a Worker's WebSocket.
#[derive(FromJs, JsType)]
pub(crate) struct NativeSocket<'js> {
    /// Delivers a message from the client.
    pub(super) receive: JsFunction<'js, (WebSocketData,)>,
    /// Delivers the client's close frame's code and reason.
    pub(super) close: JsFunction<'js, (u16, String)>,
    /// Tells the socket that the connection ended without a close frame.
    pub(super) disconnect: JsFunction<'js, ()>,
    /// The frames the Worker has sent since the last call.
    pub(super) take: JsFunction<'js, (), Vec<OutboxFrame>>,
    /// Has the socket notify the host whenever the Worker sends a frame.
    pub(super) listen: JsFunction<'js, (Class<'js, SocketNotifier>,)>,
}

/// A WebSocket message from the client: an `ArrayBuffer` when binary, else
/// its payload decoded as UTF-8.
pub(crate) struct WebSocketData {
    pub(super) binary: bool,
    pub(super) payload: Vec<u8>,
}

impl<'js> IntoJs<'js> for WebSocketData {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        if self.binary {
            ArrayBuffer::new(ctx.clone(), self.payload).map(ArrayBuffer::into_value)
        } else {
            String::from_utf8_lossy(&self.payload).into_js(ctx)
        }
    }
}

impl crate::typescript::JsType for WebSocketData {
    fn typescript(_: &mut crate::typescript::Types) -> String {
        "string | ArrayBuffer".to_owned()
    }
}

/// A frame the Worker sent on its WebSocket.
pub(crate) struct OutboxFrame(pub(super) WebSocketOutbound);

impl<'js> FromJs<'js> for OutboxFrame {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        let frame = Object::from_js(ctx, value)?;
        let frame_type: String = frame.get("type")?;
        let outbound = match frame_type.as_str() {
            "message" => {
                let binary: bool = frame.get("binary")?;
                let payload = if binary {
                    let data: ArrayBuffer = frame.get("data")?;
                    data.as_bytes()
                        .ok_or_else(|| Exception::throw_type(ctx, "WebSocket data was detached"))?
                        .to_vec()
                } else {
                    frame.get::<_, String>("data")?.into_bytes()
                };
                WebSocketOutbound::Message { binary, payload }
            }
            "close" => WebSocketOutbound::Close {
                code: frame.get("code")?,
                reason: frame.get("reason")?,
            },
            _ => return Err(Exception::throw_type(ctx, "Unknown WebSocket outbox frame")),
        };
        Ok(Self(outbound))
    }
}

impl crate::typescript::JsType for OutboxFrame {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("OutboxFrame", std::any::type_name::<Self>(), |_| {
            crate::typescript::alias(
                "OutboxFrame",
                "{ type: \"message\"; binary: true; data: ArrayBuffer } \
                 | { type: \"message\"; binary: false; data: string } \
                 | { type: \"close\"; code: number; reason: string }",
            )
        })
    }
}

/// Wakes the host when the Worker sends a WebSocket frame.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct SocketNotifier {
    #[qjs(skip_trace)]
    changed: Arc<tokio::sync::Notify>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl SocketNotifier {
    pub fn notify(&self) {
        self.changed.notify_one();
    }
}

impl SocketNotifier {
    pub(super) fn new(changed: Arc<tokio::sync::Notify>) -> Self {
        Self { changed }
    }
}

/// What a dispatched event's listeners did: their encoded reply, and the
/// names of the events that have listeners.
#[derive(FromJs)]
pub(crate) struct DispatchedEvent<'js> {
    pub(super) reply: TypedArray<'js, u8>,
    pub(super) listened: Option<Vec<String>>,
}

/// The app's assets, which the Worker's assets binding fetches from.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct AssetFetcher {
    #[qjs(skip_trace)]
    assets: Arc<Assets>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl AssetFetcher {
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn fetch<'js>(
        &self,
        ctx: Ctx<'js>,
        method: String,
        path: String,
    ) -> rquickjs::Result<AssetReply<'js>> {
        AssetReply::new(&ctx, self.assets.fetch(&method, &path))
    }
}

impl AssetFetcher {
    pub(super) fn new(assets: Arc<Assets>) -> Self {
        Self { assets }
    }
}

/// An asset response, which the assets binding builds its `Response` from.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct AssetReply<'js> {
    status: u16,
    status_text: Option<&'static str>,
    content_type: Option<String>,
    body: Option<TypedArray<'js, u8>>,
}

impl<'js> AssetReply<'js> {
    fn new(ctx: &Ctx<'js>, response: AssetResponse) -> rquickjs::Result<Self> {
        let body = response.body.map(|body| TypedArray::new(ctx.clone(), body));
        Ok(Self {
            status: response.status.as_u16(),
            status_text: response.status.canonical_reason(),
            content_type: response.content_type,
            body: body.transpose()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::{HeaderMap, HeaderValue};

    use super::RequestDescriptor;
    use crate::transport::HttpRequest;

    #[test]
    fn describes_repeated_and_non_ascii_request_headers() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut headers = HeaderMap::new();
        headers.append("x-repeated", HeaderValue::from_static("one"));
        headers.append("x-repeated", HeaderValue::from_static("two"));
        headers.append("x-text", HeaderValue::from_bytes("東京".as_bytes())?);
        let request = HttpRequest {
            persistent: false,
            method: "GET".to_owned(),
            target: "/".to_owned(),
            url: "https://app.test/".to_owned(),
            headers,
            body: None,
        };
        let pairs = RequestDescriptor::new(&request).headers.0;
        let expected = [
            ("x-repeated", "one"),
            ("x-repeated", "two"),
            ("x-text", "東京"),
        ];
        let expected = expected.map(|(name, value)| (name.to_owned(), value.to_owned()));
        assert_eq!(pairs, expected);
        Ok(())
    }
}
