#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose};
use encoding_rs::{DecoderResult, Encoding};
use rquickjs::function::MutFn;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Ctx, Exception, Function, Object, TypedArray};

pub(crate) struct HostModule;

impl ModuleDef for HostModule {
    fn declare(exports: &Declarations) -> rquickjs::Result<()> {
        for name in [
            "createDecoder",
            "encodeBase64",
            "decodeBase64",
            "randomBytes",
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
        Ok(())
    }
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
