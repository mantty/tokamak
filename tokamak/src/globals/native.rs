#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose};
use encoding_rs::{DecoderResult, Encoding};
use rquickjs::function::MutFn;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, TypedArray, Value};
use std::io::Write;

pub(crate) struct HostModule;

impl ModuleDef for HostModule {
    fn declare(exports: &Declarations) -> rquickjs::Result<()> {
        for name in [
            "asyncContextGet",
            "asyncContextSet",
            "scheduleTimer",
            "createDecoder",
            "encodeBase64",
            "decodeBase64",
            "randomBytes",
            "detachArrayBuffer",
            "objectClass",
            "cloneArrayBuffer",
            "arrayBufferView",
            "createCompression",
            "createNodeCompression",
            "htmlRewrite",
            "htmlValidateSelector",
            "httpFetch",
            "httpStatusText",
            "socketConnect",
            "ipVersion",
            "cacheMatch",
            "cachePut",
            "cacheDelete",
            "writeStdout",
            "writeStderr",
        ] {
            exports.declare(name)?;
        }
        for name in super::crypto::HOST_EXPORTS
            .iter()
            .chain(super::intl::HOST_EXPORTS)
            .chain(super::url::HOST_EXPORTS)
        {
            exports.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        super::buffers::export(ctx, exports)?;
        exports.export(
            "asyncContextGet",
            Function::new(ctx.clone(), super::async_context::get)?,
        )?;
        exports.export(
            "asyncContextSet",
            Function::new(ctx.clone(), super::async_context::set)?,
        )?;
        exports.export("createDecoder", Function::new(ctx.clone(), create_decoder)?)?;
        exports.export("scheduleTimer", super::timers::scheduler(ctx.clone())?)?;
        exports.export("encodeBase64", Function::new(ctx.clone(), encode_base64)?)?;
        exports.export("decodeBase64", Function::new(ctx.clone(), decode_base64)?)?;
        exports.export(
            "httpStatusText",
            Function::new(ctx.clone(), crate::network::http::status_text)?,
        )?;
        exports.export("randomBytes", Function::new(ctx.clone(), random_bytes)?)?;
        exports.export(
            "objectClass",
            Function::new(ctx.clone(), super::objects::class_name)?,
        )?;
        exports.export(
            "detachArrayBuffer",
            Function::new(ctx.clone(), detach_array_buffer)?,
        )?;
        exports.export(
            "createCompression",
            Function::new(ctx.clone(), super::compression::create)?,
        )?;
        exports.export(
            "createNodeCompression",
            Function::new(ctx.clone(), super::compression::node::create)?,
        )?;
        super::crypto::export_host_functions(ctx, exports)?;
        super::intl::export_host_functions(ctx, exports)?;
        super::url::export_host_functions(ctx, exports)?;
        exports.export(
            "ipVersion",
            Function::new(ctx.clone(), |input: String| {
                crate::network::sockets::ip_version(&input)
            })?,
        )?;
        exports.export(
            "htmlRewrite",
            Function::new(ctx.clone(), super::html_rewriter::rewrite_html)?,
        )?;
        exports.export(
            "htmlValidateSelector",
            Function::new(ctx.clone(), super::html_rewriter::validate_selector)?,
        )?;
        let client = crate::network::http::client()
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
        exports.export(
            "httpFetch",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>,
                      url: String,
                      method: String,
                      headers: String,
                      body: Value<'js>,
                      redirect: String| {
                    crate::network::http::start(ctx, &client, url, method, headers, body, redirect)
                },
            )?,
        )?;
        exports.export(
            "socketConnect",
            Function::new(ctx.clone(), crate::network::sockets::start)?,
        )?;
        exports.export("cacheMatch", Function::new(ctx.clone(), cache_match)?)?;
        exports.export("cachePut", Function::new(ctx.clone(), cache_put)?)?;
        exports.export("cacheDelete", Function::new(ctx.clone(), cache_delete)?)?;
        exports.export("writeStdout", Function::new(ctx.clone(), write_stdout)?)?;
        exports.export("writeStderr", Function::new(ctx.clone(), write_stderr)?)?;
        Ok(())
    }
}

fn write_stdout(text: String) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
}

fn write_stderr(text: String) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
    let _ = stderr.flush();
}

fn cache_match(
    ctx: Ctx<'_>,
    path: String,
    name: String,
    key: String,
) -> rquickjs::Result<Option<Object<'_>>> {
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
    let headers: crate::network::HeaderList = serde_json::from_str(&headers_json)
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
