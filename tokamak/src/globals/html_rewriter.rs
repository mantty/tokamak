//! `HTMLRewriter` host binding over `lol_html`.

use std::borrow::Cow;

use lol_html::html_content::{
    Comment, ContentType, Doctype, DocumentEnd, Element, EndTag, TextChunk,
};
use lol_html::{
    DocumentContentHandlers, ElementContentHandlers, HtmlRewriter, RewriteStrSettings, Selector,
};
use rquickjs::{Ctx, Exception, Function, Object};

#[derive(Debug)]
struct HtmlRewriteError(String);

impl std::fmt::Display for HtmlRewriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HtmlRewriteError {}

pub(super) fn rewrite_html<'js>(
    ctx: Ctx<'js>,
    input: String,
    handlers: rquickjs::Array<'js>,
) -> rquickjs::Result<String> {
    let mut settings = RewriteStrSettings::new();
    for entry in handlers.iter::<Object>() {
        let entry = entry?;
        let selector: String = entry.get("selector")?;
        let selector = selector
            .parse::<Selector>()
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
        if let Some(document) = entry.get::<_, Option<Object>>("document")? {
            settings = settings
                .append_document_content_handler(document_content_handlers(&ctx, &document)?);
            continue;
        }
        settings = settings.append_element_content_handler((
            Cow::Owned(selector),
            element_content_handlers(&ctx, &entry)?,
        ));
    }
    let mut output = Vec::new();
    let mut rewriter = HtmlRewriter::new(settings.into(), |chunk: &[u8]| {
        output.extend_from_slice(chunk);
    });
    rewriter
        .write(input.as_bytes())
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    rewriter
        .end()
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    String::from_utf8(output).map_err(|error| Exception::throw_type(&ctx, &error.to_string()))
}

fn document_content_handlers<'js, 'handlers>(
    ctx: &Ctx<'js>,
    document: &Object<'js>,
) -> rquickjs::Result<DocumentContentHandlers<'handlers>>
where
    'js: 'handlers,
{
    let mut handlers = DocumentContentHandlers::default();
    if let Some(handler) = document.get::<_, Option<Function>>("doctype")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.doctype(move |doctype: &mut Doctype<'_>| {
            let properties = doctype_properties(&callback_ctx, doctype).map_err(callback_error)?;
            let _: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            Ok(())
        });
    }
    if let Some(handler) = document.get::<_, Option<Function>>("comments")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.comments(move |comment: &mut Comment<'_>| {
            let properties = comment_properties(&callback_ctx, comment).map_err(callback_error)?;
            let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            apply_comment_operations(&callback_ctx, comment, &operations).map_err(callback_error)
        });
    }
    if let Some(handler) = document.get::<_, Option<Function>>("text")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.text(move |text: &mut TextChunk<'_>| {
            let properties = text_properties(&callback_ctx, text).map_err(callback_error)?;
            let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            apply_text_operations(&callback_ctx, text, &operations).map_err(callback_error)
        });
    }
    if let Some(handler) = document.get::<_, Option<Function>>("end")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.end(move |end: &mut DocumentEnd<'_>| {
            let properties = Object::new(callback_ctx.clone()).map_err(callback_error)?;
            let operations: rquickjs::Array =
                handler.call((properties,)).map_err(callback_error)?;
            apply_document_end_operations(&callback_ctx, end, &operations).map_err(callback_error)
        });
    }
    Ok(handlers)
}

fn element_content_handlers<'js, 'handlers>(
    ctx: &Ctx<'js>,
    entry: &Object<'js>,
) -> rquickjs::Result<ElementContentHandlers<'handlers>>
where
    'js: 'handlers,
{
    let mut handlers = ElementContentHandlers::default();
    if let Some(handler) = entry.get::<_, Option<Function<'js>>>("element")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.element(move |element: &mut Element<'_, '_>| {
            let properties = element_properties(&callback_ctx, element).map_err(callback_error)?;
            let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            apply_element_operations(&callback_ctx, element, &operations).map_err(callback_error)
        });
    }
    if let Some(handler) = entry.get::<_, Option<Function<'js>>>("text")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.text(move |text: &mut TextChunk<'_>| {
            let properties = text_properties(&callback_ctx, text).map_err(callback_error)?;
            let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            apply_text_operations(&callback_ctx, text, &operations).map_err(callback_error)
        });
    }
    if let Some(handler) = entry.get::<_, Option<Function<'js>>>("comments")? {
        let callback_ctx = ctx.clone();
        handlers = handlers.comments(move |comment: &mut Comment<'_>| {
            let properties = comment_properties(&callback_ctx, comment).map_err(callback_error)?;
            let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
            apply_comment_operations(&callback_ctx, comment, &operations).map_err(callback_error)
        });
    }
    Ok(handlers)
}

fn callback_error(error: rquickjs::Error) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(HtmlRewriteError(error.to_string()))
}

fn text_properties<'js>(ctx: &Ctx<'js>, text: &TextChunk<'_>) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("text", text.as_str())?;
    properties.set("lastInTextNode", text.last_in_text_node())?;
    Ok(properties)
}

fn comment_properties<'js>(
    ctx: &Ctx<'js>,
    comment: &Comment<'_>,
) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("text", comment.text())?;
    Ok(properties)
}

fn doctype_properties<'js>(
    ctx: &Ctx<'js>,
    doctype: &Doctype<'_>,
) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("name", doctype.name())?;
    properties.set("publicId", doctype.public_id())?;
    properties.set("systemId", doctype.system_id())?;
    Ok(properties)
}

fn apply_text_operations(
    ctx: &Ctx<'_>,
    text: &mut TextChunk<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<()> {
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        let content_type = operation_content_type(&operation)?;
        match name.as_str() {
            "before" => text.before(&operation.get::<_, String>("value")?, content_type),
            "after" => text.after(&operation.get::<_, String>("value")?, content_type),
            "replace" => text.replace(&operation.get::<_, String>("value")?, content_type),
            "remove" => text.remove(),
            other => {
                return Err(Exception::throw_type(
                    ctx,
                    &format!("Unknown HTML rewrite operation: {other}"),
                ));
            }
        }
    }
    Ok(())
}

fn apply_comment_operations(
    ctx: &Ctx<'_>,
    comment: &mut Comment<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<()> {
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        let content_type = operation_content_type(&operation)?;
        match name.as_str() {
            "setText" => comment
                .set_text(&operation.get::<_, String>("value")?)
                .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?,
            "before" => comment.before(&operation.get::<_, String>("value")?, content_type),
            "after" => comment.after(&operation.get::<_, String>("value")?, content_type),
            "replace" => comment.replace(&operation.get::<_, String>("value")?, content_type),
            "remove" => comment.remove(),
            other => {
                return Err(Exception::throw_type(
                    ctx,
                    &format!("Unknown HTML rewrite operation: {other}"),
                ));
            }
        }
    }
    Ok(())
}

fn apply_document_end_operations(
    ctx: &Ctx<'_>,
    end: &mut DocumentEnd<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<()> {
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        if name != "append" {
            return Err(Exception::throw_type(
                ctx,
                &format!("Unknown HTML rewrite operation: {name}"),
            ));
        }
        end.append(
            &operation.get::<_, String>("value")?,
            operation_content_type(&operation)?,
        );
    }
    Ok(())
}

fn operation_content_type(operation: &rquickjs::Object<'_>) -> rquickjs::Result<ContentType> {
    Ok(
        match operation.get::<_, Option<String>>("contentType")?.as_deref() {
            Some("html") => ContentType::Html,
            _ => ContentType::Text,
        },
    )
}

fn element_properties<'js>(
    ctx: &Ctx<'js>,
    element: &Element<'_, '_>,
) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("tagName", element.tag_name())?;
    properties.set("tagNamePreserveCase", element.tag_name_preserve_case())?;
    properties.set("namespaceURI", element.namespace_uri())?;
    properties.set("isSelfClosing", element.is_self_closing())?;
    properties.set("canHaveContent", element.can_have_content())?;
    let attributes = rquickjs::Array::new(ctx.clone())?;
    for (index, attribute) in element.attributes().iter().enumerate() {
        let value = Object::new(ctx.clone())?;
        value.set("name", attribute.name())?;
        value.set("namePreserveCase", attribute.name_preserve_case())?;
        value.set("value", attribute.value())?;
        attributes.set(index, value)?;
    }
    properties.set("attributes", attributes)?;
    Ok(properties)
}

fn apply_element_operations<'js>(
    ctx: &Ctx<'js>,
    element: &mut Element<'_, '_>,
    operations: &rquickjs::Array<'js>,
) -> rquickjs::Result<()> {
    let mut removed = false;
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        match name.as_str() {
            "setAttribute" => element
                .set_attribute(
                    &operation.get::<_, String>("attribute")?,
                    &operation.get::<_, String>("value")?,
                )
                .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?,
            "removeAttribute" => {
                element.remove_attribute(&operation.get::<_, String>("attribute")?);
            }
            "setTagName" => element
                .set_tag_name(&operation.get::<_, String>("value")?)
                .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?,
            "setInnerContent" => element.set_inner_content(
                &operation.get::<_, String>("value")?,
                operation_content_type(&operation)?,
            ),
            "before" => element.before(
                &operation.get::<_, String>("value")?,
                operation_content_type(&operation)?,
            ),
            "after" => element.after(
                &operation.get::<_, String>("value")?,
                operation_content_type(&operation)?,
            ),
            "prepend" => element.prepend(
                &operation.get::<_, String>("value")?,
                operation_content_type(&operation)?,
            ),
            "append" => element.append(
                &operation.get::<_, String>("value")?,
                operation_content_type(&operation)?,
            ),
            "replace" => {
                removed = true;
                element.replace(
                    &operation.get::<_, String>("value")?,
                    operation_content_type(&operation)?,
                );
            }
            "remove" => {
                removed = true;
                element.remove();
            }
            "removeAndKeepContent" => {
                removed = true;
                element.remove_and_keep_content();
            }
            "onEndTag" => register_end_tag_handler(ctx, element, &operation, removed)?,
            other => {
                return Err(Exception::throw_type(
                    ctx,
                    &format!("Unknown HTML rewrite operation: {other}"),
                ));
            }
        }
    }
    Ok(())
}

fn register_end_tag_handler<'js>(
    ctx: &Ctx<'js>,
    element: &mut Element<'_, '_>,
    operation: &rquickjs::Object<'js>,
    removed: bool,
) -> rquickjs::Result<()> {
    let handler: Function = operation.get("handler")?;
    let properties = Object::new(ctx.clone())?;
    properties.set("name", element.tag_name())?;
    properties.set("namePreserveCase", element.tag_name_preserve_case())?;
    properties.set("removed", removed)?;
    let callback_operations: rquickjs::Array = handler
        .call((properties,))
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let callback_operations = end_tag_operations(ctx, &callback_operations)?;
    element
        .on_end_tag(Box::new(move |end_tag| {
            apply_end_tag_operations(end_tag, &callback_operations);
            Ok(())
        }))
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))
}

enum EndTagOperation {
    Before(String, ContentType),
    After(String, ContentType),
    Remove,
}

fn end_tag_operations(
    ctx: &Ctx<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<Vec<EndTagOperation>> {
    let mut parsed = Vec::new();
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        let content_type = operation_content_type(&operation)?;
        match name.as_str() {
            "before" => parsed.push(EndTagOperation::Before(
                operation.get("value")?,
                content_type,
            )),
            "after" => parsed.push(EndTagOperation::After(
                operation.get("value")?,
                content_type,
            )),
            "remove" => parsed.push(EndTagOperation::Remove),
            other => {
                return Err(Exception::throw_type(
                    ctx,
                    &format!("Unknown HTML rewrite operation: {other}"),
                ));
            }
        }
    }
    Ok(parsed)
}

fn apply_end_tag_operations(end_tag: &mut EndTag<'_>, operations: &[EndTagOperation]) {
    for operation in operations {
        match operation {
            EndTagOperation::Before(value, content_type) => end_tag.before(value, *content_type),
            EndTagOperation::After(value, content_type) => end_tag.after(value, *content_type),
            EndTagOperation::Remove => end_tag.remove(),
        }
    }
}
