use std::borrow::Cow;
use std::io;
use std::sync::{Arc, Mutex};

use flume::{Receiver, Sender};
use lol_html::html_content::{
    Comment, ContentType, Doctype, DocumentEnd, Element, EndTag, StreamingHandler,
    StreamingHandlerSink, TextChunk,
};
use lol_html::{
    AsciiCompatibleEncoding, CommentHandler, DoctypeHandler, DocumentContentHandlers,
    ElementContentHandlers, ElementHandler, EndHandler, HtmlRewriter, MemorySettings, Selector,
    Settings, TextHandler,
};
use serde::{Deserialize, Serialize};
use tokamak_macros::JsType;

type Failure = Box<dyn std::error::Error + Send + Sync>;
type Result<T = ()> = std::result::Result<T, Failure>;

/// A registration the parser released: of an input source, or of a handler.
#[derive(Serialize, JsType)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum HtmlRelease {
    Source { id: usize },
    Handler { id: usize },
}

pub(super) type Releases = Arc<Mutex<Vec<HtmlRelease>>>;

// Streaming content and end-tag handlers can be discarded by lol_html without
// invocation. Their owned JS registrations must be released at that point too.
struct Registration {
    release: Option<HtmlRelease>,
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
    Token {
        handler: usize,
        properties: TokenProperties,
    },
    Output(Vec<u8>),
    Finished(std::result::Result<(), String>),
}

pub(super) enum Reply {
    Input(Option<Vec<u8>>),
    Operation(
        HtmlOperation,
        Sender<std::result::Result<TokenProperties, String>>,
    ),
    Done,
}

/// A token's properties, as its handler and its mutations see them.
#[derive(Serialize, JsType)]
#[serde(tag = "token", rename_all = "camelCase")]
pub(crate) enum TokenProperties {
    #[serde(rename_all = "camelCase")]
    Element {
        tag_name: String,
        #[serde(rename = "namespaceURI")]
        namespace_uri: String,
        removed: bool,
        attributes: Vec<ElementAttribute>,
    },
    #[serde(rename_all = "camelCase")]
    Text {
        text: String,
        last_in_text_node: bool,
        removed: bool,
    },
    Comment {
        text: String,
        removed: bool,
    },
    #[serde(rename_all = "camelCase")]
    Doctype {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        public_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        system_id: Option<String>,
    },
    EndTag {
        name: String,
    },
    End,
}

/// An attribute of an element.
#[derive(Serialize, JsType)]
pub(crate) struct ElementAttribute {
    name: String,
    value: String,
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

    fn registration(&self, release: HtmlRelease) -> Registration {
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

    /// Writes the chunks of the stream `source` to `sink` until it ends.
    fn write_input(
        &self,
        source: usize,
        sink: &mut StreamingHandlerSink<'_>,
        kind: ContentType,
    ) -> Result {
        while let Some(bytes) = self.input(source)? {
            sink.write_utf8_chunk(&bytes, kind)?;
        }
        Ok(())
    }

    fn call(
        &self,
        handler: usize,
        properties: TokenProperties,
        mut mutate: impl FnMut(HtmlOperation) -> Result<TokenProperties>,
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

/// A registration of handlers: on the elements `selector` matches, or on the
/// document without one. Each handler is the ID of its callback.
#[derive(Deserialize, JsType)]
pub(crate) struct HandlerSpec {
    selector: Option<String>,
    element: Option<usize>,
    text: Option<usize>,
    comments: Option<usize>,
    doctype: Option<usize>,
    end: Option<usize>,
}

pub(super) struct Handler {
    selector: Option<Selector>,
    spec: HandlerSpec,
}

pub(super) fn handlers(specs: Vec<HandlerSpec>) -> Result<Vec<Handler>> {
    specs
        .into_iter()
        .map(|spec| {
            let selector = spec.selector.as_deref().map(str::parse).transpose()?;
            Ok(Handler { selector, spec })
        })
        .collect()
}

#[derive(Clone, Copy, Deserialize, JsType)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HtmlOperationName {
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

/// Content an operation adds: a string, or the ID of an input source.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum HtmlContent {
    String(String),
    Stream { source: usize },
}

#[cfg(test)]
impl crate::typescript::JsType for HtmlContent {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("HtmlContent", std::any::type_name::<Self>(), |_| {
            crate::typescript::alias("HtmlContent", "string | { source: number }")
        })
    }
}

#[derive(Default, Clone, Copy, Deserialize, JsType)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ContentKind {
    Html,
    #[default]
    Text,
}

impl From<ContentKind> for ContentType {
    fn from(kind: ContentKind) -> Self {
        match kind {
            ContentKind::Html => Self::Html,
            ContentKind::Text => Self::Text,
        }
    }
}

/// A change to the current token.
#[derive(Deserialize, JsType)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HtmlOperation {
    name: HtmlOperationName,
    value: Option<HtmlContent>,
    attribute: Option<String>,
    handler: Option<usize>,
    #[serde(default)]
    content_type: ContentKind,
}

impl HtmlOperation {
    fn string(&self) -> Result<&str> {
        match &self.value {
            Some(HtmlContent::String(value)) => Ok(value),
            _ => Err(invalid("This HTML operation requires a string")),
        }
    }

    fn attribute_name(&self) -> Result<&str> {
        self.attribute
            .as_deref()
            .ok_or_else(|| invalid("Missing attribute name"))
    }

    fn stream(self, port: &Port) -> Result<Box<dyn StreamingHandler + Send + 'static>> {
        let content = self.value.ok_or_else(|| invalid("Missing HTML content"))?;
        let kind = self.content_type.into();
        let port = port.clone();
        let registration = match &content {
            HtmlContent::Stream { source } => {
                Some(port.registration(HtmlRelease::Source { id: *source }))
            }
            HtmlContent::String(_) => None,
        };
        Ok(Box::new(move |sink: &mut StreamingHandlerSink<'_>| {
            let _registration = registration;
            match content {
                HtmlContent::String(value) => sink.write_str(&value, kind),
                HtmlContent::Stream { source } => port.write_input(source, sink, kind)?,
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
        settings = match selector {
            Some(selector) => settings.append_element_content_handler((
                Cow::Owned(selector),
                element_handlers(&spec, port),
            )),
            None => settings.append_document_content_handler(document_handlers(&spec, port)),
        };
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

fn element_handlers<'h>(spec: &HandlerSpec, port: &'h Port) -> ElementContentHandlers<'h> {
    ElementContentHandlers {
        element: element_handler(spec, port),
        comments: comment_handler(spec, port),
        text: text_handler(spec, port),
    }
}

fn document_handlers<'h>(spec: &HandlerSpec, port: &'h Port) -> DocumentContentHandlers<'h> {
    DocumentContentHandlers {
        doctype: doctype_handler(spec, port),
        comments: comment_handler(spec, port),
        text: text_handler(spec, port),
        end: end_handler(spec, port),
    }
}

fn element_handler<'h>(spec: &HandlerSpec, port: &'h Port) -> Option<ElementHandler<'h>> {
    let handler = spec.element?;
    Some(Box::new(move |element: &mut Element<'_, '_>| {
        port.call(handler, element_properties(element), |operation| {
            apply_element(element, operation, port)?;
            Ok(element_properties(element))
        })
    }))
}

fn doctype_handler<'h>(spec: &HandlerSpec, port: &'h Port) -> Option<DoctypeHandler<'h>> {
    let handler = spec.doctype?;
    Some(Box::new(move |doctype: &mut Doctype<'_>| {
        let properties = TokenProperties::Doctype {
            name: doctype.name(),
            public_id: doctype.public_id(),
            system_id: doctype.system_id(),
        };
        port.call(handler, properties, |_| {
            Err(invalid("Doctype is read-only"))
        })
    }))
}

fn end_handler<'h>(spec: &HandlerSpec, port: &'h Port) -> Option<EndHandler<'h>> {
    let handler = spec.end?;
    Some(Box::new(move |end: &mut DocumentEnd<'_>| {
        port.call(handler, TokenProperties::End, |operation| {
            apply_end(end, &operation)?;
            Ok(TokenProperties::End)
        })
    }))
}

fn text_handler<'h>(spec: &HandlerSpec, port: &'h Port) -> Option<TextHandler<'h>> {
    let handler = spec.text?;
    Some(Box::new(move |text: &mut TextChunk<'_>| {
        port.call(handler, text_properties(text), |operation| {
            apply_text(text, operation, port)?;
            Ok(text_properties(text))
        })
    }))
}

fn comment_handler<'h>(spec: &HandlerSpec, port: &'h Port) -> Option<CommentHandler<'h>> {
    let handler = spec.comments?;
    Some(Box::new(move |comment: &mut Comment<'_>| {
        port.call(handler, comment_properties(comment), |operation| {
            apply_comment(comment, &operation)?;
            Ok(comment_properties(comment))
        })
    }))
}

fn text_properties(text: &TextChunk<'_>) -> TokenProperties {
    TokenProperties::Text {
        text: text.as_str().to_owned(),
        last_in_text_node: text.last_in_text_node(),
        removed: text.removed(),
    }
}

fn comment_properties(comment: &Comment<'_>) -> TokenProperties {
    TokenProperties::Comment {
        text: comment.text(),
        removed: comment.removed(),
    }
}

fn element_properties(element: &Element<'_, '_>) -> TokenProperties {
    let attributes = element
        .attributes()
        .iter()
        .map(|attribute| ElementAttribute {
            name: attribute.name(),
            value: attribute.value(),
        });
    TokenProperties::Element {
        tag_name: element.tag_name(),
        namespace_uri: element.namespace_uri().to_owned(),
        removed: element.removed(),
        attributes: attributes.collect(),
    }
}

fn apply_element(element: &mut Element<'_, '_>, operation: HtmlOperation, port: &Port) -> Result {
    match operation.name {
        HtmlOperationName::SetAttribute => {
            element.set_attribute(operation.attribute_name()?, operation.string()?)?;
        }
        HtmlOperationName::RemoveAttribute => element.remove_attribute(operation.attribute_name()?),
        HtmlOperationName::SetTagName => element.set_tag_name(operation.string()?)?,
        HtmlOperationName::SetInnerContent => {
            element.streaming_set_inner_content(operation.stream(port)?);
        }
        HtmlOperationName::Before => element.streaming_before(operation.stream(port)?),
        HtmlOperationName::After => element.streaming_after(operation.stream(port)?),
        HtmlOperationName::Prepend => element.streaming_prepend(operation.stream(port)?),
        HtmlOperationName::Append => element.streaming_append(operation.stream(port)?),
        HtmlOperationName::Replace => element.streaming_replace(operation.stream(port)?),
        HtmlOperationName::Remove => element.remove(),
        HtmlOperationName::RemoveAndKeepContent => element.remove_and_keep_content(),
        HtmlOperationName::OnEndTag => {
            let handler = operation
                .handler
                .ok_or_else(|| invalid("Missing end-tag handler"))?;
            let port = port.clone();
            let registration = port.registration(HtmlRelease::Handler { id: handler });
            if let Some(handlers) = element.end_tag_handlers() {
                handlers.clear();
            }
            element.on_end_tag(Box::new(move |end_tag| {
                let _registration = registration;
                let name = end_tag.name();
                port.call(handler, TokenProperties::EndTag { name }, |operation| {
                    apply_end_tag(end_tag, operation, &port)?;
                    Ok(TokenProperties::EndTag {
                        name: end_tag.name(),
                    })
                })
            }))?;
        }
        HtmlOperationName::SetText => return Err(invalid("Element does not support setText")),
    }
    Ok(())
}

fn apply_text(text: &mut TextChunk<'_>, operation: HtmlOperation, port: &Port) -> Result {
    match operation.name {
        HtmlOperationName::Before => text.streaming_before(operation.stream(port)?),
        HtmlOperationName::After => text.streaming_after(operation.stream(port)?),
        HtmlOperationName::Replace => text.streaming_replace(operation.stream(port)?),
        HtmlOperationName::Remove => text.remove(),
        _ => return Err(invalid("Invalid text operation")),
    }
    Ok(())
}

fn apply_comment(comment: &mut Comment<'_>, operation: &HtmlOperation) -> Result {
    match operation.name {
        HtmlOperationName::SetText => comment.set_text(operation.string()?)?,
        HtmlOperationName::Before => {
            comment.before(operation.string()?, operation.content_type.into());
        }
        HtmlOperationName::After => {
            comment.after(operation.string()?, operation.content_type.into());
        }
        HtmlOperationName::Replace => {
            comment.replace(operation.string()?, operation.content_type.into());
        }
        HtmlOperationName::Remove => comment.remove(),
        _ => return Err(invalid("Invalid comment operation")),
    }
    Ok(())
}

fn apply_end_tag(end: &mut EndTag<'_>, operation: HtmlOperation, port: &Port) -> Result {
    match operation.name {
        HtmlOperationName::Before => end.streaming_before(operation.stream(port)?),
        HtmlOperationName::After => end.streaming_after(operation.stream(port)?),
        HtmlOperationName::Remove => end.remove(),
        _ => return Err(invalid("Invalid end-tag operation")),
    }
    Ok(())
}

fn apply_end(end: &mut DocumentEnd<'_>, operation: &HtmlOperation) -> Result {
    if !matches!(operation.name, HtmlOperationName::Append) {
        return Err(invalid("Invalid document-end operation"));
    }
    end.append(operation.string()?, operation.content_type.into());
    Ok(())
}
