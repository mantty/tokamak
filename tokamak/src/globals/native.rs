#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose};
use encoding_rs::{DecoderResult, Encoding};
use rquickjs::function::MutFn;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    ArrayBuffer, Constructor, Ctx, Exception, Function, JsLifetime, Object, TypedArray, Value,
};
use std::io::Write;

use crate::runtime_modules;

pub(crate) struct HostModule;

super::host_functions! {
    pub(super),
    "asyncContextGet" => super::async_context::get,
    "asyncContextSet" => super::async_context::set,
    "createDecoder" => create_decoder,
    "encodeBase64" => encode_base64,
    "decodeBase64" => decode_base64,
    "randomBytes" => random_bytes,
    "detachArrayBuffer" => detach_array_buffer,
    "objectClass" => super::objects::class_name,
    "createCompression" => super::compression::create,
    "createNodeCompression" => super::compression::node::create,
    "htmlRewrite" => super::html_rewriter::rewrite_html,
    "htmlValidateSelector" => super::html_rewriter::validate_selector,
    "httpStatusText" => crate::network::http::status_text,
    "socketConnect" => crate::network::sockets::start,
    "ipVersion" => |input: String| crate::network::sockets::ip_version(&input),
    "cacheMatch" => crate::cache::cache_match,
    "cachePut" => crate::cache::cache_put,
    "cacheDelete" => crate::cache::cache_delete,
    "writeStdout" => |text: String| write_flushed(&mut std::io::stdout().lock(), &text),
    "writeStderr" => |text: String| write_flushed(&mut std::io::stderr().lock(), &text),
    "registerDomException" => register_dom_exception,
    "builtinNamespace" => |ctx: Ctx<'js>, name: String| runtime_modules::builtin_namespace(ctx, &name),
    "isNodeBuiltin" => |name: String| runtime_modules::is_node_builtin(&name),
    "nodeBuiltinNames" => runtime_modules::node_builtin_names,
}

/// The `DOMException` class native errors are created with, registered by the
/// module that defines it. A runtime has one context, so one class.
#[derive(JsLifetime)]
struct DomException<'js>(Constructor<'js>);

fn register_dom_exception<'js>(
    ctx: Ctx<'js>,
    constructor: Constructor<'js>,
) -> rquickjs::Result<()> {
    ctx.store_userdata(DomException(constructor))
        .map(drop)
        .map_err(|_| Exception::throw_internal(&ctx, "DOMException could not be registered"))
}

/// A `DOMException` named `name`, thrown in `ctx`.
pub(super) fn throw_dom_exception(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let Some(constructor) = ctx.userdata::<DomException<'_>>() else {
        return Exception::throw_internal(ctx, &format!("{name}: {message}"));
    };
    match constructor.0.construct::<_, Object>((message, name)) {
        Ok(exception) => ctx.throw(exception.into()),
        Err(error) => error,
    }
}

/// Exports that `evaluate` constructs itself.
const CONSTRUCTED_EXPORTS: [&str; 4] = [
    "cloneArrayBuffer",
    "arrayBufferView",
    "scheduleTimer",
    "httpFetch",
];

impl ModuleDef for HostModule {
    fn declare(exports: &Declarations) -> rquickjs::Result<()> {
        for name in CONSTRUCTED_EXPORTS
            .iter()
            .chain(HOST_EXPORTS)
            .chain(super::crypto::HOST_EXPORTS)
            .chain(super::intl::HOST_EXPORTS)
            .chain(super::url::HOST_EXPORTS)
        {
            exports.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        super::buffers::export(ctx, exports)?;
        exports.export("scheduleTimer", super::timers::scheduler(ctx.clone())?)?;
        export_host_functions(ctx, exports)?;
        super::crypto::export_host_functions(ctx, exports)?;
        super::intl::export_host_functions(ctx, exports)?;
        super::url::export_host_functions(ctx, exports)?;
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
        Ok(())
    }
}

fn write_flushed(output: &mut dyn Write, text: &str) {
    let _ = output.write_all(text.as_bytes());
    let _ = output.flush();
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

fn detach_array_buffer(mut buffer: ArrayBuffer<'_>) {
    buffer.detach();
}
