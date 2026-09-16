use std::borrow::Cow;
use std::io;
use std::sync::{Arc, Mutex};

use flume::{Receiver, Sender};
use lol_html::html_content::{
    Comment, ContentType, Doctype, DocumentEnd, Element, EndTag, StreamingHandler,
    StreamingHandlerSink, TextChunk,
};
use lol_html::{
    AsciiCompatibleEncoding, DocumentContentHandlers, ElementContentHandlers, HtmlRewriter,
    MemorySettings, Selector, Settings,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

type Failure = Box<dyn std::error::Error + Send + Sync>;
type Result<T = ()> = std::result::Result<T, Failure>;

#[derive(Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub(super) enum Release {
    Source(usize),
    Handler(usize),
}

pub(super) type Releases = Arc<Mutex<Vec<Release>>>;

// Streaming content and end-tag handlers can be discarded by lol_html without
// invocation. Their owned JS registrations must be released at that point too.
struct Registration {
    release: Option<Release>,
    queue: Releases,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            self.queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(release);
        }
    }
}

fn invalid(message: &str) -> Failure {
    io::Error::other(message).into()
}

pub(super) enum Event {
    Input(usize),
    Token { handler: usize, properties: Value },
    Output(Vec<u8>),
    Finished(std::result::Result<(), String>),
}

pub(super) enum Reply {
    Input(Option<Vec<u8>>),
    Operation(Operation, Sender<std::result::Result<Value, String>>),
    Done,
}

#[derive(Clone)]
pub(super) struct Port {
    events: Sender<Event>,
    replies: Receiver<Reply>,
    stop: Receiver<()>,
    releases: Releases,
}

impl Port {
    pub(super) fn new(
        events: Sender<Event>,
        replies: Receiver<Reply>,
        stop: Receiver<()>,
        releases: Releases,
    ) -> Self {
        Self {
            events,
            replies,
            stop,
            releases,
        }
    }

    fn registration(&self, release: Release) -> Registration {
        Registration {
            release: Some(release),
            queue: Arc::clone(&self.releases),
        }
    }

    pub(super) fn send(&self, event: Event) -> Result {
        if self.stop.is_disconnected() {
            return Err(invalid("HTML rewriting cancelled"));
        }
        flume::Selector::new()
            .send(&self.events, event, |sent| {
                sent.map_err(|_| invalid("HTML output closed"))
            })
            .recv(&self.stop, |_| Err(invalid("HTML rewriting cancelled")))
            .wait()
    }

    fn reply(&self) -> Result<Reply> {
        flume::Selector::new()
            .recv(&self.replies, |reply| {
                reply.map_err(|_| invalid("HTML input closed"))
            })
            .recv(&self.stop, |_| Err(invalid("HTML rewriting cancelled")))
            .wait()
    }

    fn input(&self, source: usize) -> Result<Option<Vec<u8>>> {
        self.send(Event::Input(source))?;
        match self.reply()? {
            Reply::Input(bytes) => Ok(bytes),
            _ => Err(invalid("Expected HTML input bytes")),
        }
    }

    fn call(
        &self,
        handler: usize,
        properties: Value,
        mut mutate: impl FnMut(Operation) -> Result<Value>,
    ) -> Result {
        self.send(Event::Token {
            handler,
            properties,
        })?;
        loop {
            match self.reply()? {
                Reply::Done => return Ok(()),
                Reply::Operation(operation, result) => {
                    let value = mutate(operation).map_err(|error| error.to_string());
                    // The caller may have dropped its runtime during a mutation.
                    let _ = result.send(value);
                }
                Reply::Input(_) => return Err(invalid("Expected HTML token operation")),
            }
        }
    }
}

#[derive(Deserialize)]
struct Spec {
    selector: Option<String>,
    element: Option<usize>,
    text: Option<usize>,
    comments: Option<usize>,
    doctype: Option<usize>,
    end: Option<usize>,
}

pub(super) struct Handler {
    selector: Option<Selector>,
    spec: Spec,
}

pub(super) fn handlers(input: &str) -> Result<Vec<Handler>> {
    serde_json::from_str::<Vec<Spec>>(input)?
        .into_iter()
        .map(|spec| {
            let selector = spec.selector.as_deref().map(str::parse).transpose()?;
            Ok(Handler { selector, spec })
        })
        .collect()
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum OperationName {
    Before,
    After,
    Replace,
    Remove,
    SetText,
    SetAttribute,
    RemoveAttribute,
    SetTagName,
    SetInnerContent,
    Prepend,
    Append,
    RemoveAndKeepContent,
    OnEndTag,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    String(String),
    Stream { source: usize },
}

#[derive(Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Html,
    #[default]
    Text,
}

impl From<Kind> for ContentType {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Html => Self::Html,
            Kind::Text => Self::Text,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Operation {
    name: OperationName,
    value: Option<Content>,
    attribute: Option<String>,
    handler: Option<usize>,
    #[serde(default)]
    content_type: Kind,
}

impl Operation {
    fn string(&self) -> Result<&str> {
        match &self.value {
            Some(Content::String(value)) => Ok(value),
            _ => Err(invalid("This HTML operation requires a string")),
        }
    }

    fn stream(self, port: &Port) -> Result<Box<dyn StreamingHandler + Send + 'static>> {
        let content = self.value.ok_or_else(|| invalid("Missing HTML content"))?;
        let kind = self.content_type.into();
        let port = port.clone();
        let registration = match &content {
            Content::Stream { source } => Some(port.registration(Release::Source(*source))),
            Content::String(_) => None,
        };
        Ok(Box::new(move |sink: &mut StreamingHandlerSink<'_>| {
            let _registration = registration;
            match content {
                Content::String(value) => sink.write_str(&value, kind),
                Content::Stream { source } => {
                    while let Some(bytes) = port.input(source)? {
                        sink.write_utf8_chunk(&bytes, kind)?;
                    }
                }
            }
            Ok(())
        }))
    }
}

pub(super) fn run(
    handlers: Vec<Handler>,
    encoding: AsciiCompatibleEncoding,
    port: &Port,
) -> Result {
    let mut settings = Settings::new()
        .with_encoding(encoding)
        .with_memory_settings(MemorySettings::new().with_max_allowed_memory_usage(3 * 1024 * 1024));
    for Handler { selector, spec } in handlers {
        if let Some(selector) = selector {
            let mut handlers = ElementContentHandlers::default();
            if let Some(handler) = spec.element {
                handlers = handlers.element(move |element: &mut Element<'_, '_>| {
                    port.call(handler, element_properties(element), |operation| {
                        apply_element(element, operation, port)?;
                        Ok(element_properties(element))
                    })
                });
            }
            if let Some(handler) = spec.text {
                handlers = handlers.text(move |text: &mut TextChunk<'_>| {
                    port.call(handler, text_properties(text), |operation| {
                        apply_text(text, operation, port)?;
                        Ok(text_properties(text))
                    })
                });
            }
            if let Some(handler) = spec.comments {
                handlers = handlers.comments(move |comment: &mut Comment<'_>| {
                    port.call(handler, comment_properties(comment), |operation| {
                        apply_comment(comment, &operation)?;
                        Ok(comment_properties(comment))
                    })
                });
            }
            settings = settings.append_element_content_handler((Cow::Owned(selector), handlers));
        } else {
            let mut handlers = DocumentContentHandlers::default();
            if let Some(handler) = spec.doctype {
                handlers = handlers.doctype(move |doctype: &mut Doctype<'_>| {
                    port.call(handler, json!({"name": doctype.name(), "publicId": doctype.public_id(), "systemId": doctype.system_id()}), |_| Err(invalid("Doctype is read-only")))
                });
            }
            if let Some(handler) = spec.text {
                handlers = handlers.text(move |text: &mut TextChunk<'_>| {
                    port.call(handler, text_properties(text), |operation| {
                        apply_text(text, operation, port)?;
                        Ok(text_properties(text))
                    })
                });
            }
            if let Some(handler) = spec.comments {
                handlers = handlers.comments(move |comment: &mut Comment<'_>| {
                    port.call(handler, comment_properties(comment), |operation| {
                        apply_comment(comment, &operation)?;
                        Ok(comment_properties(comment))
                    })
                });
            }
            if let Some(handler) = spec.end {
                handlers = handlers.end(move |end: &mut DocumentEnd<'_>| {
                    port.call(handler, json!({}), |operation| {
                        apply_end(end, &operation)?;
                        Ok(json!({}))
                    })
                });
            }
            settings = settings.append_document_content_handler(handlers);
        }
    }
    let mut rewriter = HtmlRewriter::new(settings, |bytes: &[u8]| {
        for bytes in bytes.chunks(16384) {
            if port.send(Event::Output(bytes.to_vec())).is_err() {
                break;
            }
        }
    });
    while let Some(bytes) = port.input(0)? {
        rewriter.write(&bytes)?;
    }
    rewriter.end()?;
    Ok(())
}

fn text_properties(text: &TextChunk<'_>) -> Value {
    json!({"text": text.as_str(), "lastInTextNode": text.last_in_text_node(), "removed": text.removed()})
}

fn comment_properties(comment: &Comment<'_>) -> Value {
    json!({"text": comment.text(), "removed": comment.removed()})
}

fn element_properties(element: &Element<'_, '_>) -> Value {
    json!({
        "tagName": element.tag_name(), "namespaceURI": element.namespace_uri(),
        "removed": element.removed(),
        "attributes": element.attributes().iter().map(|attribute| json!({
            "name": attribute.name(), "value": attribute.value(),
        })).collect::<Vec<_>>(),
    })
}

fn apply_element(element: &mut Element<'_, '_>, operation: Operation, port: &Port) -> Result {
    match operation.name {
        OperationName::SetAttribute => element.set_attribute(
            operation
                .attribute
                .as_deref()
                .ok_or_else(|| invalid("Missing attribute name"))?,
            operation.string()?,
        )?,
        OperationName::RemoveAttribute => element.remove_attribute(
            operation
                .attribute
                .as_deref()
                .ok_or_else(|| invalid("Missing attribute name"))?,
        ),
        OperationName::SetTagName => element.set_tag_name(operation.string()?)?,
        OperationName::SetInnerContent => {
            element.streaming_set_inner_content(operation.stream(port)?);
        }
        OperationName::Before => element.streaming_before(operation.stream(port)?),
        OperationName::After => element.streaming_after(operation.stream(port)?),
        OperationName::Prepend => element.streaming_prepend(operation.stream(port)?),
        OperationName::Append => element.streaming_append(operation.stream(port)?),
        OperationName::Replace => element.streaming_replace(operation.stream(port)?),
        OperationName::Remove => element.remove(),
        OperationName::RemoveAndKeepContent => element.remove_and_keep_content(),
        OperationName::OnEndTag => {
            let handler = operation
                .handler
                .ok_or_else(|| invalid("Missing end-tag handler"))?;
            let port = port.clone();
            let registration = port.registration(Release::Handler(handler));
            if let Some(handlers) = element.end_tag_handlers() {
                handlers.clear();
            }
            element.on_end_tag(Box::new(move |end_tag| {
                let _registration = registration;
                port.call(handler, json!({"name": end_tag.name()}), |operation| {
                    apply_end_tag(end_tag, operation, &port)?;
                    Ok(json!({"name": end_tag.name()}))
                })
            }))?;
        }
        OperationName::SetText => return Err(invalid("Element does not support setText")),
    }
    Ok(())
}

fn apply_text(text: &mut TextChunk<'_>, operation: Operation, port: &Port) -> Result {
    match operation.name {
        OperationName::Before => text.streaming_before(operation.stream(port)?),
        OperationName::After => text.streaming_after(operation.stream(port)?),
        OperationName::Replace => text.streaming_replace(operation.stream(port)?),
        OperationName::Remove => text.remove(),
        _ => return Err(invalid("Invalid text operation")),
    }
    Ok(())
}

fn apply_comment(comment: &mut Comment<'_>, operation: &Operation) -> Result {
    match operation.name {
        OperationName::SetText => comment.set_text(operation.string()?)?,
        OperationName::Before => comment.before(operation.string()?, operation.content_type.into()),
        OperationName::After => comment.after(operation.string()?, operation.content_type.into()),
        OperationName::Replace => {
            comment.replace(operation.string()?, operation.content_type.into());
        }
        OperationName::Remove => comment.remove(),
        _ => return Err(invalid("Invalid comment operation")),
    }
    Ok(())
}

fn apply_end_tag(end: &mut EndTag<'_>, operation: Operation, port: &Port) -> Result {
    match operation.name {
        OperationName::Before => end.streaming_before(operation.stream(port)?),
        OperationName::After => end.streaming_after(operation.stream(port)?),
        OperationName::Remove => end.remove(),
        _ => return Err(invalid("Invalid end-tag operation")),
    }
    Ok(())
}

fn apply_end(end: &mut DocumentEnd<'_>, operation: &Operation) -> Result {
    if !matches!(operation.name, OperationName::Append) {
        return Err(invalid("Invalid document-end operation"));
    }
    end.append(operation.string()?, operation.content_type.into());
    Ok(())
}
