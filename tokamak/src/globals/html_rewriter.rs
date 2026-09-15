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
use rquickjs::function::Async;
use rquickjs::{Ctx, Exception, Function, Object, TypedArray, Value};
use serde_json::json;

use parser::{Event, Handler, Port, Releases, Reply};

#[allow(clippy::needless_pass_by_value)]
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

#[allow(clippy::needless_pass_by_value)]
pub(super) fn rewrite_html<'js>(
    ctx: Ctx<'js>,
    handlers: String,
    content_type: String,
) -> rquickjs::Result<Object<'js>> {
    let handlers = parser::handlers(&handlers)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let encoding = parse_encoding(&ctx, &content_type)?;
    let state = Rc::new(RefCell::new(State {
        handlers: Some(handlers),
        encoding,
        channels: None,
        ended: false,
        releases: Releases::default(),
    }));
    let result = Object::new(ctx.clone())?;
    let reader = Rc::clone(&state);
    result.set(
        "next",
        Function::new(
            ctx.clone(),
            Async(move |ctx: Ctx<'js>| next_event(ctx, Rc::clone(&reader))),
        )?,
    )?;
    let writer = Rc::clone(&state);
    result.set(
        "reply",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, value: Value<'js>| {
            let reply = decode_reply(&ctx, &value)?;
            let state = writer.borrow();
            if state.ended {
                return Ok(());
            }
            state
                .channels
                .as_ref()
                .and_then(|channels| channels.replies.as_ref())
                .ok_or_else(|| Exception::throw_internal(&ctx, "HTML parser is not running"))?
                .try_send(reply)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
        })?,
    )?;
    let mutator = Rc::clone(&state);
    result.set(
        "mutate",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, operation: String| {
            mutate(&ctx, &mutator.borrow(), &operation)
        })?,
    )?;
    let registry = Rc::clone(&state);
    result.set(
        "releases",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            let released = std::mem::take(
                &mut *registry
                    .borrow()
                    .releases
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            ctx.json_parse(
                serde_json::to_string(&released)
                    .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?,
            )
        })?,
    )?;
    result.set(
        "cancel",
        Function::new(ctx, move |ctx: Ctx<'js>| {
            state
                .borrow_mut()
                .stop()
                .map_err(|error| Exception::throw_internal(&ctx, &error))
        })?,
    )?;
    Ok(result)
}

fn mutate<'js>(ctx: &Ctx<'js>, state: &State, operation: &str) -> rquickjs::Result<Value<'js>> {
    let operation = serde_json::from_str(operation)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let (result, response) = flume::bounded(1);
    state
        .channels
        .as_ref()
        .and_then(|channels| channels.replies.as_ref())
        .ok_or_else(|| Exception::throw_type(ctx, "HTML parser is not running"))?
        .try_send(Reply::Operation(operation, result))
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    let value = response
        .recv()
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?
        .map_err(|error| Exception::throw_type(ctx, &error))?;
    ctx.json_parse(value.to_string())
}

fn parse_encoding(ctx: &Ctx<'_>, content_type: &str) -> rquickjs::Result<AsciiCompatibleEncoding> {
    let mime = content_type.parse::<mime_guess::Mime>().ok();
    let label = mime.as_ref().and_then(|mime| mime.get_param("charset"));
    let label = label.as_ref().map_or("utf-8", |label| label.as_str());
    encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
        .and_then(AsciiCompatibleEncoding::new)
        .ok_or_else(|| {
            Exception::throw_type(ctx, "HTMLRewriter requires an ASCII-compatible encoding")
        })
}

fn decode_reply(ctx: &Ctx<'_>, value: &Value<'_>) -> rquickjs::Result<Reply> {
    Ok(if value.is_undefined() {
        Reply::Done
    } else if value.is_null() {
        Reply::Input(None)
    } else if let Some(bytes) = value
        .clone()
        .into_object()
        .and_then(|object| TypedArray::<u8>::from_object(object).ok())
    {
        Reply::Input(Some(
            bytes
                .as_bytes()
                .ok_or_else(|| Exception::throw_type(ctx, "Detached HTML input"))?
                .to_vec(),
        ))
    } else {
        return Err(Exception::throw_type(ctx, "Invalid HTML parser reply"));
    })
}

async fn next_event(ctx: Ctx<'_>, state: Rc<RefCell<State>>) -> rquickjs::Result<Value<'_>> {
    let receiver = state
        .borrow_mut()
        .events()
        .map_err(|error| Exception::throw_internal(&ctx, &error))?;
    let Some(receiver) = receiver else {
        return Ok(Value::new_null(ctx));
    };
    let event = receiver
        .recv_async()
        .await
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    match event {
        Event::Input(source) => {
            ctx.json_parse(json!({"kind": "input", "source": source}).to_string())
        }
        Event::Token {
            handler,
            properties,
        } => ctx.json_parse(
            json!({
                "kind": "token", "handler": handler, "properties": properties,
            })
            .to_string(),
        ),
        Event::Output(bytes) => {
            let event = Object::new(ctx.clone())?;
            event.set("kind", "output")?;
            event.set("bytes", TypedArray::new(ctx, bytes)?)?;
            Ok(event.into_value())
        }
        Event::Finished(result) => {
            state
                .borrow_mut()
                .stop()
                .map_err(|error| Exception::throw_internal(&ctx, &error))?;
            result.map_err(|error| Exception::throw_type(&ctx, &error))?;
            Ok(Value::new_null(ctx))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    fn take_releases(state: &State) -> serde_json::Value {
        let released = std::mem::take(
            &mut *state
                .releases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        serde_json::to_value(released).unwrap_or_default()
    }

    #[test]
    fn discarded_registrations_are_released_before_the_next_token() -> TestResult {
        for tag in ["p", "br"] {
            let mut state = State {
                handlers: Some(parser::handlers(r#"[{"selector":"*","element":0}]"#)?),
                encoding: AsciiCompatibleEncoding::utf_8(),
                channels: None,
                ended: false,
                releases: Releases::default(),
            };
            let events = state.events()?.ok_or("event channel closed")?;
            let channels = state.channels.as_ref().ok_or("channels missing")?;
            let replies = channels.replies.as_ref().ok_or("replies missing")?;
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
                let expected = if tag == "br" {
                    Some(handler)
                } else if handler > 1 {
                    Some(handler - 1)
                } else {
                    None
                };
                assert_eq!(
                    take_releases(&state),
                    expected.map_or(json!([]), |id| json!([{"kind":"handler","id":id}]))
                );
            }
            state.stop()?;
            assert_eq!(take_releases(&state).as_array().map_or(0, Vec::len), usize::from(tag == "p"));
        }
        Ok(())
    }

    #[test]
    fn dropping_rewriter_joins_at_each_blocking_boundary() -> TestResult {
        for boundary in ["input", "token", "output"] {
            let (finished, completion) = flume::bounded(1);
            let check = thread::spawn(move || -> Result<(), String> {
                let handlers = if boundary == "token" {
                    r#"[{"selector":"p","element":0}]"#
                } else {
                    "[]"
                };
                let mut state = State {
                    handlers: Some(parser::handlers(handlers).map_err(|error| error.to_string())?),
                    encoding: AsciiCompatibleEncoding::utf_8(),
                    channels: None,
                    ended: false,
                    releases: Releases::default(),
                };
                let events = state
                    .events()?
                    .ok_or_else(|| "event channel closed".to_owned())?;
                let first = events
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|error| error.to_string())?;
                assert!(matches!(first, Event::Input(0)));
                if boundary != "input" {
                    let bytes = if boundary == "token" {
                        b"<p>hello</p>".to_vec()
                    } else {
                        vec![b'x'; 65536]
                    };
                    let channels = state.channels.as_ref().ok_or_else(|| "channels missing".to_owned())?;
                    channels
                        .replies
                        .as_ref()
                        .ok_or_else(|| "replies missing".to_owned())?
                        .send(Reply::Input(Some(bytes)))
                        .map_err(|error| error.to_string())?;
                    let event = events
                        .recv_timeout(Duration::from_secs(2))
                        .map_err(|error| error.to_string())?;
                    assert!(matches!(
                        (boundary, event),
                        ("token", Event::Token { .. }) | ("output", Event::Output(_))
                    ));
                }
                // Keep a receiver alive: merely dropping the output receiver must
                // not be necessary to interrupt a blocked send or handler reply.
                drop(state);
                while events.recv_timeout(Duration::from_secs(2)).is_ok() {}
                assert!(events.is_disconnected());
                finished.send(()).map_err(|error| error.to_string())
            });
            completion
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "HTML parser did not stop and join")?;
            check.join().map_err(|_| "check thread panicked")??;
        }
        Ok(())
    }
}
