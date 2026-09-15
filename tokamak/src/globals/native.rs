#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose};
use encoding_rs::{DecoderResult, Encoding};
use rquickjs::function::MutFn;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, TypedArray};
use std::io::{Read, Write};

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
            "writeStdout",
            "writeStderr",
        ] {
            exports.declare(name)?;
        }
        for name in super::crypto::HOST_EXPORTS
            .iter()
            .chain(super::intl::HOST_EXPORTS)
        {
            exports.declare(*name)?;
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
        super::crypto::export_host_functions(ctx, exports)?;
        super::intl::export_host_functions(ctx, exports)?;
        exports.export("htmlRewrite", Function::new(ctx.clone(), super::html_rewriter::rewrite_html)?)?;
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

