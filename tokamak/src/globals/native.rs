#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose};
use encoding_rs::{DecoderResult, Encoding};
use rquickjs::function::MutFn;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, TypedArray};
use std::collections::BTreeMap;
use std::io::Read;

pub(crate) struct HostModule;

impl ModuleDef for HostModule {
    fn declare(exports: &Declarations) -> rquickjs::Result<()> {
        for name in [
            "createDecoder",
            "encodeBase64",
            "decodeBase64",
            "randomBytes",
            "detachArrayBuffer",
            "compress",
            "decompress",
            "digest",
            "cryptoHmac",
            "cryptoAesGcm",
            "intlCanonicalLocales",
            "intlDateTime",
            "intlDateTimeParts",
            "intlNumber",
            "intlNumberParts",
            "intlPlural",
            "intlPluralRange",
            "intlPluralCategories",
            "intlList",
            "intlListParts",
            "intlRelative",
            "intlRelativeParts",
            "intlCollatorCompare",
            "intlSegment",
            "intlDisplayName",
            "intlLocaleInfo",
            "htmlRewrite",
            "httpFetch",
            "socketConnect",
            "socketStartTls",
            "socketRead",
            "socketWrite",
            "socketClose",
            "cacheMatch",
            "cachePut",
            "cacheDelete",
        ] {
            exports.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        exports.export("createDecoder", Function::new(ctx.clone(), create_decoder)?)?;
        exports.export("encodeBase64", Function::new(ctx.clone(), encode_base64)?)?;
        exports.export("decodeBase64", Function::new(ctx.clone(), decode_base64)?)?;
        exports.export("randomBytes", Function::new(ctx.clone(), random_bytes)?)?;
        exports.export(
            "detachArrayBuffer",
            Function::new(ctx.clone(), detach_array_buffer)?,
        )?;
        exports.export("compress", Function::new(ctx.clone(), compress)?)?;
        exports.export("decompress", Function::new(ctx.clone(), decompress)?)?;
        exports.export("digest", Function::new(ctx.clone(), super::crypto::digest)?)?;
        exports.export(
            "cryptoHmac",
            Function::new(ctx.clone(), super::crypto::hmac)?,
        )?;
        exports.export(
            "cryptoAesGcm",
            Function::new(ctx.clone(), super::crypto::aes_gcm)?,
        )?;
        exports.export(
            "intlCanonicalLocales",
            Function::new(ctx.clone(), super::intl::canonical_locales)?,
        )?;
        exports.export(
            "intlDateTime",
            Function::new(ctx.clone(), super::intl::date_time)?,
        )?;
        exports.export(
            "intlDateTimeParts",
            Function::new(ctx.clone(), super::intl::date_time_parts)?,
        )?;
        exports.export(
            "intlNumber",
            Function::new(ctx.clone(), super::intl::number)?,
        )?;
        exports.export(
            "intlNumberParts",
            Function::new(ctx.clone(), super::intl::number_parts)?,
        )?;
        exports.export(
            "intlPlural",
            Function::new(ctx.clone(), super::intl::plural)?,
        )?;
        exports.export(
            "intlPluralRange",
            Function::new(ctx.clone(), super::intl::plural_range)?,
        )?;
        exports.export(
            "intlPluralCategories",
            Function::new(ctx.clone(), super::intl::plural_categories)?,
        )?;
        exports.export("intlList", Function::new(ctx.clone(), super::intl::list)?)?;
        exports.export(
            "intlListParts",
            Function::new(ctx.clone(), super::intl::list_parts)?,
        )?;
        exports.export(
            "intlRelative",
            Function::new(ctx.clone(), super::intl::relative)?,
        )?;
        exports.export(
            "intlRelativeParts",
            Function::new(ctx.clone(), super::intl::relative_parts)?,
        )?;
        exports.export(
            "intlCollatorCompare",
            Function::new(ctx.clone(), super::intl::collator_compare)?,
        )?;
        exports.export(
            "intlSegment",
            Function::new(ctx.clone(), super::intl::segment)?,
        )?;
        exports.export(
            "intlDisplayName",
            Function::new(ctx.clone(), super::intl::display_name)?,
        )?;
        exports.export(
            "intlLocaleInfo",
            Function::new(ctx.clone(), super::intl::locale_info)?,
        )?;
        exports.export("htmlRewrite", Function::new(ctx.clone(), rewrite_html)?)?;
        exports.export("httpFetch", Function::new(ctx.clone(), http_fetch)?)?;
        exports.export(
            "socketConnect",
            Function::new(ctx.clone(), socket_connect)?,
        )?;
        exports.export(
            "socketStartTls",
            Function::new(ctx.clone(), socket_start_tls)?,
        )?;
        exports.export("socketRead", Function::new(ctx.clone(), socket_read)?)?;
        exports.export("socketWrite", Function::new(ctx.clone(), socket_write)?)?;
        exports.export("socketClose", Function::new(ctx.clone(), socket_close)?)?;
        exports.export("cacheMatch", Function::new(ctx.clone(), cache_match)?)?;
        exports.export("cachePut", Function::new(ctx.clone(), cache_put)?)?;
        exports.export("cacheDelete", Function::new(ctx.clone(), cache_delete)?)?;
        Ok(())
    }
}

fn network_exception(ctx: &Ctx<'_>, error: crate::network::Error) -> rquickjs::Error {
    Exception::throw_type(ctx, &error.to_string())
}

fn socket_exception(ctx: &Ctx<'_>, error: crate::network::Error) -> rquickjs::Error {
    Exception::throw_message(ctx, &error.to_string())
}

fn http_fetch<'js>(
    ctx: Ctx<'js>,
    url: String,
    method: String,
    headers_json: String,
    body: TypedArray<'js, u8>,
    redirect: String,
) -> rquickjs::Result<Object<'js>> {
    let headers = serde_json::from_str(&headers_json)
        .map_err(|error| Exception::throw_type(&ctx, &format!("Invalid request headers: {error}")))?;
    let body = body
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
        .to_vec();
    let response = crate::network::fetch(crate::network::FetchRequest {
        url,
        method,
        headers,
        body,
        redirect,
    })
    .map_err(|error| network_exception(&ctx, error))?;
    let result = Object::new(ctx.clone())?;
    result.set("url", response.url)?;
    result.set("status", response.status)?;
    result.set("statusText", response.status_text)?;
    result.set(
        "headers",
        serde_json::to_string(&response.headers)
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?,
    )?;
    result.set("body", TypedArray::new(ctx.clone(), response.body)?)?;
    result.set("redirected", response.redirected)?;
    Ok(result)
}

fn socket_connect(ctx: Ctx<'_>, host: String, port: u16, secure: bool) -> rquickjs::Result<u64> {
    crate::network::socket_connect(&host, port, secure).map_err(|error| socket_exception(&ctx, error))
}

fn socket_start_tls(ctx: Ctx<'_>, id: u64, host: String) -> rquickjs::Result<()> {
    crate::network::socket_start_tls(id, &host).map_err(|error| socket_exception(&ctx, error))
}

fn socket_read<'js>(ctx: Ctx<'js>, id: u64) -> rquickjs::Result<Option<TypedArray<'js, u8>>> {
    crate::network::socket_read(id)
        .map_err(|error| socket_exception(&ctx, error))?
        .map(|bytes| TypedArray::new(ctx.clone(), bytes))
        .transpose()
}

fn socket_write(ctx: Ctx<'_>, id: u64, data: TypedArray<'_, u8>) -> rquickjs::Result<()> {
    let data = data
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    crate::network::socket_write(id, data).map_err(|error| socket_exception(&ctx, error))
}

fn socket_close(_: Ctx<'_>, id: u64) {
    crate::network::socket_close(id);
}

fn cache_match<'js>(
    ctx: Ctx<'js>,
    path: String,
    name: String,
    key: String,
) -> rquickjs::Result<Option<Object<'js>>> {
    let Some(entry) = crate::network::cache_match(&path, &name, &key) else {
        return Ok(None);
    };
    let result = Object::new(ctx.clone())?;
    result.set("status", entry.status)?;
    result.set("statusText", entry.status_text)?;
    result.set(
        "headers",
        serde_json::to_string(&entry.headers)
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?,
    )?;
    result.set("body", TypedArray::new(ctx.clone(), entry.body)?)?;
    result.set("url", entry.url)?;
    result.set("redirected", entry.redirected)?;
    result.set("type", entry.response_type)?;
    Ok(Some(result))
}

fn cache_put<'js>(
    ctx: Ctx<'js>,
    path: String,
    name: String,
    key: String,
    metadata: Object<'js>,
    body: TypedArray<'js, u8>,
) -> rquickjs::Result<()> {
    let status: u16 = metadata.get("status")?;
    let status_text: String = metadata.get("statusText")?;
    let headers_json: String = metadata.get("headers")?;
    let url: String = metadata.get("url")?;
    let redirected: bool = metadata.get("redirected")?;
    let response_type: String = metadata.get("type")?;
    let headers: BTreeMap<String, String> = serde_json::from_str(&headers_json)
        .map_err(|error| Exception::throw_type(&ctx, &format!("Invalid cache headers: {error}")))?;
    let body = body
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?
        .to_vec();
    crate::network::cache_put(
        &path,
        &name,
        key,
        crate::network::CacheEntry {
            status,
            status_text,
            headers,
            body,
            url,
            redirected,
            response_type,
        },
    );
    Ok(())
}

fn cache_delete(_: Ctx<'_>, path: String, name: String, key: String) -> bool {
    crate::network::cache_delete(&path, &name, &key)
}

fn create_decoder<'js>(
    ctx: Ctx<'js>,
    label: String,
    fatal: bool,
    ignore_bom: bool,
) -> rquickjs::Result<Object<'js>> {
    let encoding = Encoding::for_label_no_replacement(label.as_bytes())
        .ok_or_else(|| Exception::throw_range(&ctx, "Unsupported encoding"))?;
    let unicode = [
        encoding_rs::UTF_8,
        encoding_rs::UTF_16LE,
        encoding_rs::UTF_16BE,
    ]
    .contains(&encoding);
    let new_decoder = move || {
        if ignore_bom {
            encoding.new_decoder_without_bom_handling()
        } else {
            encoding.new_decoder_with_bom_removal()
        }
    };
    let mut decoder = new_decoder();
    let object = Object::new(ctx.clone())?;
    object.set("encoding", encoding.name().to_ascii_lowercase())?;
    let decode = Function::new(
        ctx,
        MutFn::new(
            move |ctx: Ctx<'js>,
                  input: TypedArray<'js, u8>,
                  stream: bool|
                  -> rquickjs::Result<String> {
                let bytes = input
                    .as_bytes()
                    .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
                let capacity = decoder
                    .max_utf8_buffer_length(bytes.len())
                    .ok_or_else(|| Exception::throw_range(&ctx, "Input is too large"))?;
                let mut output = String::with_capacity(capacity);
                let malformed = if fatal {
                    let (status, _) =
                        decoder.decode_to_string_without_replacement(bytes, &mut output, !stream);
                    matches!(status, DecoderResult::Malformed(_, _))
                } else {
                    let _ = decoder.decode_to_string(bytes, &mut output, !stream);
                    false
                };
                // Workerd preserves Unicode stream state after a fatal error.
                if !stream || (malformed && !unicode) {
                    decoder = new_decoder();
                }
                if malformed {
                    return Err(Exception::throw_type(&ctx, "Invalid encoded data"));
                }
                Ok(output)
            },
        ),
    )?;
    object.set("decode", decode)?;
    Ok(object)
}

fn encode_base64(ctx: Ctx<'_>, bytes: TypedArray<'_, u8>) -> rquickjs::Result<String> {
    let bytes = bytes
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    Ok(general_purpose::STANDARD.encode(bytes))
}

fn decode_base64(ctx: Ctx<'_>, input: String) -> rquickjs::Result<Option<TypedArray<'_, u8>>> {
    let engine = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine
        .decode(input)
        .ok()
        .map(|bytes| TypedArray::new(ctx, bytes))
        .transpose()
}

fn random_bytes(ctx: Ctx<'_>, length: usize) -> rquickjs::Result<TypedArray<'_, u8>> {
    if length > 65_536 {
        return Err(Exception::throw_range(
            &ctx,
            "Random data exceeds 65536 bytes",
        ));
    }
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    TypedArray::new(ctx, bytes)
}

fn detach_array_buffer(_: Ctx<'_>, mut buffer: ArrayBuffer<'_>) {
    buffer.detach();
}

fn compress<'js>(
    ctx: Ctx<'js>,
    format: String,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<TypedArray<'js, u8>> {
    use flate2::write::{DeflateEncoder, ZlibEncoder};
    use flate2::{Compression, GzBuilder};
    use std::io::Write;

    let input = input
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    let output = match format.as_str() {
        "gzip" => {
            let mut encoder = GzBuilder::new()
                .operating_system(19)
                .write(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            encoder
                .finish()
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
        }
        "deflate" => {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            encoder
                .finish()
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
        }
        "deflate-raw" => {
            let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(input)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            encoder
                .finish()
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
        }
        "brotli" => {
            let mut encoder = brotli::CompressorReader::new(input, 4096, 11, 22);
            let mut output = Vec::new();
            encoder
                .read_to_end(&mut output)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
            output
        }
        "zstd" => zstd::stream::encode_all(input, 0)
            .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?,
        _ => {
            return Err(Exception::throw_type(
                &ctx,
                "Unsupported compression format",
            ));
        }
    };
    TypedArray::new(ctx, output)
}

fn decompress<'js>(
    ctx: Ctx<'js>,
    format: String,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<TypedArray<'js, u8>> {
    use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
    use std::io::Read;

    let input = input
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    let mut output = Vec::new();
    match format.as_str() {
        "gzip" => GzDecoder::new(input)
            .read_to_end(&mut output)
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
        "deflate" => ZlibDecoder::new(input)
            .read_to_end(&mut output)
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
        "deflate-raw" => DeflateDecoder::new(input)
            .read_to_end(&mut output)
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
        "brotli" => brotli::Decompressor::new(input, 4096)
            .read_to_end(&mut output)
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
        "zstd" => {
            output = zstd::stream::decode_all(input)
                .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
            0
        }
        _ => {
            return Err(Exception::throw_type(
                &ctx,
                "Unsupported compression format",
            ));
        }
    };
    TypedArray::new(ctx, output)
}

#[derive(Debug)]
struct HtmlRewriteError(String);

impl std::fmt::Display for HtmlRewriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HtmlRewriteError {}

fn rewrite_html<'js>(
    ctx: Ctx<'js>,
    input: String,
    handlers: rquickjs::Array<'js>,
) -> rquickjs::Result<String> {
    use lol_html::html_content::{Comment, Doctype, DocumentEnd, Element, TextChunk};
    use lol_html::{DocumentContentHandlers, ElementContentHandlers, HtmlRewriter, RewriteStrSettings, Selector};
    use std::borrow::Cow;

    let mut settings = RewriteStrSettings::new();
    for entry in handlers.iter::<Object>() {
        let entry = entry?;
        let selector: String = entry.get("selector")?;
        let selector = selector
            .parse::<Selector>()
            .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
        if let Some(document) = entry.get::<_, Option<Object>>("document")? {
            let mut document_handlers = DocumentContentHandlers::default();
            if let Some(handler) = document.get::<_, Option<Function>>("doctype")? {
                let callback_ctx = ctx.clone();
                document_handlers = document_handlers.doctype(move |doctype: &mut Doctype<'_>| {
                    let properties = doctype_properties(&callback_ctx, doctype).map_err(callback_error)?;
                    let _: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
                    Ok(())
                });
            }
            if let Some(handler) = document.get::<_, Option<Function>>("comments")? {
                let callback_ctx = ctx.clone();
                document_handlers = document_handlers.comments(move |comment: &mut Comment<'_>| {
                    let properties = comment_properties(&callback_ctx, comment).map_err(callback_error)?;
                    let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
                    apply_comment_operations(&callback_ctx, comment, &operations).map_err(callback_error)
                });
            }
            if let Some(handler) = document.get::<_, Option<Function>>("text")? {
                let callback_ctx = ctx.clone();
                document_handlers = document_handlers.text(move |text: &mut TextChunk<'_>| {
                    let properties = text_properties(&callback_ctx, text).map_err(callback_error)?;
                    let operations: rquickjs::Array = handler.call((properties,)).map_err(callback_error)?;
                    apply_text_operations(&callback_ctx, text, &operations).map_err(callback_error)
                });
            }
            if let Some(handler) = document.get::<_, Option<Function>>("end")? {
                let callback_ctx = ctx.clone();
                document_handlers = document_handlers.end(move |end: &mut DocumentEnd<'_>| {
                    let operations: rquickjs::Array = handler.call((Object::new(callback_ctx.clone()).map_err(callback_error)?,)).map_err(callback_error)?;
                    apply_document_end_operations(&callback_ctx, end, &operations).map_err(callback_error)
                });
            }
            settings = settings.append_document_content_handler(document_handlers);
            continue;
        }
        let element_handler: Option<Function<'js>> = entry.get("element")?;
        let text_handler: Option<Function<'js>> = entry.get("text")?;
        let comment_handler: Option<Function<'js>> = entry.get("comments")?;
        let mut content_handlers = ElementContentHandlers::default();
        if let Some(element_handler) = element_handler {
            let callback_ctx = ctx.clone();
            content_handlers = content_handlers.element(move |element: &mut Element<'_, '_>| {
                let properties = element_properties(&callback_ctx, element).map_err(callback_error)?;
                let operations: rquickjs::Array = element_handler.call((properties,)).map_err(callback_error)?;
                apply_element_operations(&callback_ctx, element, &operations).map_err(callback_error)
            });
        }
        if let Some(text_handler) = text_handler {
            let callback_ctx = ctx.clone();
            content_handlers = content_handlers.text(move |text: &mut TextChunk<'_>| {
                let properties = text_properties(&callback_ctx, text).map_err(callback_error)?;
                let operations: rquickjs::Array = text_handler.call((properties,)).map_err(callback_error)?;
                apply_text_operations(&callback_ctx, text, &operations).map_err(callback_error)
            });
        }
        if let Some(comment_handler) = comment_handler {
            let callback_ctx = ctx.clone();
            content_handlers = content_handlers.comments(move |comment: &mut Comment<'_>| {
                let properties = comment_properties(&callback_ctx, comment).map_err(callback_error)?;
                let operations: rquickjs::Array = comment_handler.call((properties,)).map_err(callback_error)?;
                apply_comment_operations(&callback_ctx, comment, &operations).map_err(callback_error)
            });
        }
        settings =
            settings.append_element_content_handler((Cow::Owned(selector), content_handlers));
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

fn callback_error(error: rquickjs::Error) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(HtmlRewriteError(error.to_string()))
}

fn text_properties<'js>(ctx: &Ctx<'js>, text: &lol_html::html_content::TextChunk<'_>) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("text", text.as_str())?;
    properties.set("lastInTextNode", text.last_in_text_node())?;
    Ok(properties)
}

fn comment_properties<'js>(ctx: &Ctx<'js>, comment: &lol_html::html_content::Comment<'_>) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("text", comment.text())?;
    Ok(properties)
}

fn doctype_properties<'js>(ctx: &Ctx<'js>, doctype: &lol_html::html_content::Doctype<'_>) -> rquickjs::Result<Object<'js>> {
    let properties = Object::new(ctx.clone())?;
    properties.set("name", doctype.name())?;
    properties.set("publicId", doctype.public_id())?;
    properties.set("systemId", doctype.system_id())?;
    Ok(properties)
}

fn apply_text_operations(
    ctx: &Ctx<'_>,
    text: &mut lol_html::html_content::TextChunk<'_>,
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
            other => return Err(Exception::throw_type(ctx, &format!("Unknown HTML rewrite operation: {other}"))),
        }
    }
    Ok(())
}

fn apply_comment_operations(
    ctx: &Ctx<'_>,
    comment: &mut lol_html::html_content::Comment<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<()> {
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        let content_type = operation_content_type(&operation)?;
        match name.as_str() {
            "setText" => comment.set_text(&operation.get::<_, String>("value")?).map_err(|error| Exception::throw_type(ctx, &error.to_string()))?,
            "before" => comment.before(&operation.get::<_, String>("value")?, content_type),
            "after" => comment.after(&operation.get::<_, String>("value")?, content_type),
            "replace" => comment.replace(&operation.get::<_, String>("value")?, content_type),
            "remove" => comment.remove(),
            other => return Err(Exception::throw_type(ctx, &format!("Unknown HTML rewrite operation: {other}"))),
        }
    }
    Ok(())
}

fn apply_document_end_operations(
    ctx: &Ctx<'_>,
    end: &mut lol_html::html_content::DocumentEnd<'_>,
    operations: &rquickjs::Array<'_>,
) -> rquickjs::Result<()> {
    for operation in operations.iter::<rquickjs::Object>() {
        let operation = operation?;
        let name: String = operation.get("name")?;
        if name != "append" {
            return Err(Exception::throw_type(ctx, &format!("Unknown HTML rewrite operation: {name}")));
        }
        end.append(&operation.get::<_, String>("value")?, operation_content_type(&operation)?);
    }
    Ok(())
}

fn operation_content_type(operation: &rquickjs::Object<'_>) -> rquickjs::Result<lol_html::html_content::ContentType> {
    Ok(match operation.get::<_, Option<String>>("contentType")?.as_deref() {
        Some("html") => lol_html::html_content::ContentType::Html,
        _ => lol_html::html_content::ContentType::Text,
    })
}

fn element_properties<'js>(
    ctx: &Ctx<'js>,
    element: &lol_html::html_content::Element<'_, '_>,
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
    element: &mut lol_html::html_content::Element<'_, '_>,
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
            "setInnerContent" => {
                element.set_inner_content(
                    &operation.get::<_, String>("value")?,
                    operation_content_type(&operation)?,
                )
            }
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
                )
            }
            "remove" => {
                removed = true;
                element.remove()
            }
            "removeAndKeepContent" => {
                removed = true;
                element.remove_and_keep_content()
            }
            "onEndTag" => {
                let handler: Function = operation.get("handler")?;
                let properties = Object::new(ctx.clone())?;
                properties.set("name", element.tag_name())?;
                properties.set("namePreserveCase", element.tag_name_preserve_case())?;
                properties.set("removed", removed)?;
                let callback_operations: rquickjs::Array =
                    handler
                        .call((properties,))
                        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
                let callback_operations = end_tag_operations(ctx, &callback_operations)?;
                element
                    .on_end_tag(Box::new(move |end_tag| {
                        apply_end_tag_operations(end_tag, &callback_operations);
                        Ok(())
                    }))
                    .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
            }
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

enum EndTagOperation {
    Before(String, lol_html::html_content::ContentType),
    After(String, lol_html::html_content::ContentType),
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
            other => return Err(Exception::throw_type(ctx, &format!("Unknown HTML rewrite operation: {other}"))),
        }
    }
    Ok(parsed)
}

fn apply_end_tag_operations(
    end_tag: &mut lol_html::html_content::EndTag<'_>,
    operations: &[EndTagOperation],
) {
    for operation in operations {
        match operation {
            EndTagOperation::Before(value, content_type) => end_tag.before(value, *content_type),
            EndTagOperation::After(value, content_type) => end_tag.after(value, *content_type),
            EndTagOperation::Remove => end_tag.remove(),
        }
    }
}
