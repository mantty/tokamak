//! Streaming HTML rewriting. `lol_html` borrows each token during its synchronous
//! callback; a parser thread preserves that borrow while JS awaits its handler.
//! Only owned data crosses the bounded channels. JavaScript stays on its runtime
//! thread. Cancellation closes the protocol and joins the parser, including when
//! the request's JS context is dropped.

mod parser;

use std::cell::RefCell;
use std::rc::Rc;
use std::thread::{self, JoinHandle};

use flume::{Receiver, Sender};
use lol_html::AsciiCompatibleEncoding;
use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, Exception, IntoJs, JsLifetime, Object, TypedArray, Value};
use tokamak_macros::host_function;

use crate::typed::{JsPromise, Serde};
use parser::{
    Event, Handler, HandlerSpec, HtmlOperation, HtmlRelease, Port, Releases, Reply, TokenProperties,
};

#[allow(clippy::needless_pass_by_value)]
#[host_function]
pub(super) fn validate_selector(ctx: Ctx<'_>, selector: String) -> rquickjs::Result<()> {
    selector
        .parse::<lol_html::Selector>()
        .map(|_| ())
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))
}

struct Channels {
    events: Option<Receiver<Event>>,
    replies: Option<Sender<Reply>>,
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Channels {
    fn stop(&mut self) -> Result<(), String> {
        self.stop.take();
        self.replies.take();
        self.events.take();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| "HTML parser thread panicked".to_owned())?;
        }
        Ok(())
    }
}

impl Drop for Channels {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("tokamak: {error}");
        }
    }
}

struct State {
    handlers: Option<Vec<Handler>>,
    encoding: AsciiCompatibleEncoding,
    channels: Option<Channels>,
    ended: bool,
    releases: Releases,
}

impl State {
    fn new(handlers: Vec<Handler>, encoding: AsciiCompatibleEncoding) -> Self {
        Self {
            handlers: Some(handlers),
            encoding,
            channels: None,
            ended: false,
            releases: Releases::default(),
        }
    }

    /// The registrations the parser released since the last call.
    fn take_releases(&self) -> Vec<HtmlRelease> {
        std::mem::take(
            &mut *self
                .releases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Where replies to the running parser go.
    fn replies(&self) -> Option<&Sender<Reply>> {
        self.channels
            .as_ref()
            .and_then(|channels| channels.replies.as_ref())
    }

    fn events(&mut self) -> Result<Option<Receiver<Event>>, String> {
        if self.ended {
            return Ok(None);
        }
        if let Some(handlers) = self.handlers.take() {
            let (event_tx, event_rx) = flume::bounded(1);
            let (reply_tx, reply_rx) = flume::bounded(1);
            let (stop_tx, stop_rx) = flume::bounded(0);
            let port = Port::new(event_tx, reply_rx, stop_rx, self.releases.clone());
            let encoding = self.encoding;
            let parser_thread = thread::Builder::new()
                .name("tokamak-html".to_owned())
                .spawn(move || {
                    let result =
                        parser::run(handlers, encoding, &port).map_err(|error| error.to_string());
                    let _ = port.send(Event::Finished(result));
                })
                .map_err(|error| error.to_string())?;
            self.channels = Some(Channels {
                events: Some(event_rx),
                replies: Some(reply_tx),
                stop: Some(stop_tx),
                thread: Some(parser_thread),
            });
        }
        Ok(self
            .channels
            .as_ref()
            .and_then(|channels| channels.events.clone()))
    }

    fn stop(&mut self) -> Result<(), String> {
        self.ended = true;
        self.handlers.take();
        if let Some(mut channels) = self.channels.take() {
            channels.stop()?;
        }
        Ok(())
    }
}

/// Starts rewriting HTML with the callbacks `handlers` name, decoding it as
/// `content_type` says.
#[allow(clippy::needless_pass_by_value)]
#[host_function]
pub(super) fn rewrite_html(
    ctx: Ctx<'_>,
    handlers: Serde<Vec<HandlerSpec>>,
    content_type: String,
) -> rquickjs::Result<Class<'_, HtmlParser>> {
    let handlers = parser::handlers(handlers.0)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let encoding = parse_encoding(&ctx, &content_type)?;
    let state = Rc::new(RefCell::new(State::new(handlers, encoding)));
    Class::instance(ctx, HtmlParser { state })
}

/// A rewrite in progress, which the runtime drives: it answers each input
/// event with `input`, and each token event with `done` once the token's
/// handler settles.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct HtmlParser {
    #[qjs(skip_trace)]
    state: Rc<RefCell<State>>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl HtmlParser {
    /// Resolves to the parser's next event, or undefined once it finishes.
    pub fn next<'js>(
        &self,
        ctx: Ctx<'js>,
    ) -> rquickjs::Result<JsPromise<'js, Option<ParserEvent<'js>>>> {
        let state = Rc::clone(&self.state);
        JsPromise::wrap_future(&ctx.clone(), next_event(ctx, state))
    }

    /// Answers an input event with the source's next bytes, or undefined at
    /// its end.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn input<'js>(
        &self,
        ctx: Ctx<'js>,
        bytes: Option<TypedArray<'js, u8>>,
    ) -> rquickjs::Result<()> {
        let bytes = bytes
            .map(|bytes| {
                bytes
                    .as_bytes()
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| Exception::throw_type(&ctx, "Detached HTML input"))
            })
            .transpose()?;
        self.reply(&ctx, Reply::Input(bytes))
    }

    /// Answers a token event once the token's handler has settled.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn done(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.reply(&ctx, Reply::Done)
    }

    /// Applies `operation` to the current token, returning its properties
    /// after the change.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn mutate(
        &self,
        ctx: Ctx<'_>,
        operation: Serde<HtmlOperation>,
    ) -> rquickjs::Result<Serde<TokenProperties>> {
        let (result, response) = flume::bounded(1);
        self.state
            .borrow()
            .replies()
            .ok_or_else(|| Exception::throw_type(&ctx, "HTML parser is not running"))?
            .try_send(Reply::Operation(operation.0, result))
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
        let properties = response
            .recv()
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
            .map_err(|error| Exception::throw_type(&ctx, &error))?;
        Ok(Serde(properties))
    }

    /// The registrations the parser released since the last call.
    pub fn releases(&self) -> Serde<Vec<HtmlRelease>> {
        Serde(self.state.borrow().take_releases())
    }

    /// Stops the parser.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn cancel(&self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        self.state
            .borrow_mut()
            .stop()
            .map_err(|error| Exception::throw_internal(&ctx, &error))
    }
}

impl HtmlParser {
    /// Sends `reply` to the parser unless it has ended.
    fn reply(&self, ctx: &Ctx<'_>, reply: Reply) -> rquickjs::Result<()> {
        let state = self.state.borrow();
        if state.ended {
            return Ok(());
        }
        state
            .replies()
            .ok_or_else(|| Exception::throw_internal(ctx, "HTML parser is not running"))?
            .try_send(reply)
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))
    }
}

/// What the parser asks of the runtime: the next bytes of an input source,
/// a run of a token's handler, or the delivery of rewritten bytes.
pub(crate) enum ParserEvent<'js> {
    Input {
        source: usize,
    },
    Token {
        handler: usize,
        properties: Serde<TokenProperties>,
    },
    Output(TypedArray<'js, u8>),
}

impl<'js> IntoJs<'js> for ParserEvent<'js> {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let event = Object::new(ctx.clone())?;
        match self {
            Self::Input { source } => {
                event.set("kind", "input")?;
                event.set("source", source)?;
            }
            Self::Token {
                handler,
                properties,
            } => {
                event.set("kind", "token")?;
                event.set("handler", handler)?;
                event.set("properties", properties)?;
            }
            Self::Output(bytes) => {
                event.set("kind", "output")?;
                event.set("bytes", bytes)?;
            }
        }
        Ok(event.into_value())
    }
}

impl crate::typescript::JsType for ParserEvent<'_> {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("ParserEvent", std::any::type_name::<Self>(), |types| {
            let kind = |name: &str| ("kind", false, format!("{name:?}"));
            let events = [
                crate::typescript::object(&[kind("input"), ("source", false, "number".to_owned())]),
                crate::typescript::object(&[
                    kind("token"),
                    ("handler", false, "number".to_owned()),
                    ("properties", false, TokenProperties::typescript(types)),
                ]),
                crate::typescript::object(&[
                    kind("output"),
                    ("bytes", false, "Uint8Array".to_owned()),
                ]),
            ];
            crate::typescript::alias("ParserEvent", &events.join(" | "))
        })
    }
}

fn parse_encoding(ctx: &Ctx<'_>, content_type: &str) -> rquickjs::Result<AsciiCompatibleEncoding> {
    let mime = content_type.parse::<mime::Mime>().ok();
    let label = mime.as_ref().and_then(|mime| mime.get_param("charset"));
    let label = label.map_or("utf-8", |label| label.as_str());
    encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
        .and_then(AsciiCompatibleEncoding::new)
        .ok_or_else(|| {
            Exception::throw_type(ctx, "HTMLRewriter requires an ASCII-compatible encoding")
        })
}

async fn next_event(
    ctx: Ctx<'_>,
    state: Rc<RefCell<State>>,
) -> rquickjs::Result<Option<ParserEvent<'_>>> {
    let receiver = state
        .borrow_mut()
        .events()
        .map_err(|error| Exception::throw_internal(&ctx, &error))?;
    let Some(receiver) = receiver else {
        return Ok(None);
    };
    let event = receiver
        .recv_async()
        .await
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    Ok(Some(match event {
        Event::Input(source) => ParserEvent::Input { source },
        Event::Token {
            handler,
            properties,
        } => ParserEvent::Token {
            handler,
            properties: Serde(properties),
        },
        Event::Output(bytes) => ParserEvent::Output(TypedArray::new(ctx, bytes)?),
        Event::Finished(result) => {
            state
                .borrow_mut()
                .stop()
                .map_err(|error| Exception::throw_internal(&ctx, &error))?;
            result.map_err(|error| Exception::throw_type(&ctx, &error))?;
            return Ok(None);
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// The handlers the runtime keeps after registering `handler` on `tag`.
    fn expected_release(tag: &str, handler: i32) -> serde_json::Value {
        let kept = if tag == "br" {
            Some(handler)
        } else if handler > 1 {
            Some(handler - 1)
        } else {
            None
        };
        kept.map_or(json!([]), |id| json!([{"kind":"handler","id":id}]))
    }

    fn released(state: &State) -> serde_json::Value {
        serde_json::to_value(state.take_releases()).unwrap_or_default()
    }

    #[test]
    fn discarded_registrations_are_released_before_the_next_token() -> TestResult {
        for tag in ["p", "br"] {
            let mut state = State::new(
                parser::handlers(serde_json::from_str(r#"[{"selector":"*","element":0}]"#)?)?,
                AsciiCompatibleEncoding::utf_8(),
            );
            let events = state.events()?.ok_or("event channel closed")?;
            let replies = state.replies().ok_or("replies missing")?;
            assert!(matches!(events.recv()?, Event::Input(0)));
            replies
                .send(Reply::Input(Some(format!("<{tag}>").into_bytes())))
                .map_err(|_| "input send failed")?;
            assert!(matches!(events.recv()?, Event::Token { .. }));
            for handler in 1..=1000 {
                let operation =
                    serde_json::from_value(json!({"name": "onEndTag", "handler": handler}))?;
                let (result, response) = flume::bounded(1);
                replies
                    .send(Reply::Operation(operation, result))
                    .map_err(|_| "operation send failed")?;
                let accepted = response.recv_timeout(Duration::from_secs(2))?.is_ok();
                assert_eq!(accepted, tag == "p");
                assert_eq!(released(&state), expected_release(tag, handler));
            }
            state.stop()?;
            assert_eq!(
                released(&state).as_array().map_or(0, Vec::len),
                usize::from(tag == "p")
            );
        }
        Ok(())
    }

    #[test]
    fn dropping_rewriter_joins_at_each_blocking_boundary() -> TestResult {
        for boundary in ["input", "token", "output"] {
            let (finished, completion) = flume::bounded(1);
            let check = thread::spawn(move || -> Result<(), String> {
                drop_blocked_rewriter(boundary)?;
                finished.send(()).map_err(|error| error.to_string())
            });
            completion
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "HTML parser did not stop and join")?;
            check.join().map_err(|_| "check thread panicked")??;
        }
        Ok(())
    }

    /// Drops a rewriter blocked at `boundary` and checks that its parser stops.
    fn drop_blocked_rewriter(boundary: &str) -> Result<(), String> {
        let (handlers, input) = match boundary {
            "input" => ("[]", None),
            "token" => (
                r#"[{"selector":"p","element":0}]"#,
                Some(b"<p>hello</p>".to_vec()),
            ),
            _ => ("[]", Some(vec![b'x'; 65536])),
        };
        let mut state = State::new(
            serde_json::from_str(handlers)
                .map_err(|error| error.to_string())
                .and_then(|specs| parser::handlers(specs).map_err(|error| error.to_string()))?,
            AsciiCompatibleEncoding::utf_8(),
        );
        let events = state
            .events()?
            .ok_or_else(|| "event channel closed".to_owned())?;
        let first = events
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| error.to_string())?;
        assert!(matches!(first, Event::Input(0)));
        if let Some(bytes) = input {
            state
                .replies()
                .ok_or_else(|| "replies missing".to_owned())?
                .send(Reply::Input(Some(bytes)))
                .map_err(|error| error.to_string())?;
            let event = events
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| error.to_string())?;
            assert!(
                matches!(
                    (boundary, event),
                    ("token", Event::Token { .. }) | ("output", Event::Output(_))
                ),
                "expected a {boundary} event"
            );
        }
        // Keep a receiver alive: merely dropping the output receiver must
        // not be necessary to interrupt a blocked send or handler reply.
        drop(state);
        while events.recv_timeout(Duration::from_secs(2)).is_ok() {}
        assert!(events.is_disconnected());
        Ok(())
    }
}
