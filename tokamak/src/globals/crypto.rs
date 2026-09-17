#![allow(clippy::needless_pass_by_value)]

mod aes;
mod bignum;
mod cipher;
mod curves;
mod digest;
mod hash;
mod keys;
mod rsa;

use rquickjs::module::Exports;
use rquickjs::{ArrayBuffer, Constructor, Ctx, Exception, Function, Object, TypedArray};
use serde_json::Value as JsonValue;
use subtle::ConstantTimeEq;

use self::aes::Mode;
use self::curves::Curve;
use self::hash::Hash;
use self::keys::{KeyError, KeyMaterial, Kind, PrivateKey, PublicKey};

pub(super) const HOST_EXPORTS: &[&str] = &[
    "digest",
    "cryptoCreateDigest",
    "cryptoHmac",
    "cryptoAesGcm",
    "cryptoCreateCipher",
    "cryptoTimingSafeEqual",
    "cryptoPbkdf2",
    "cryptoHkdf",
    "cryptoGenerateKey",
    "cryptoImportKey",
    "cryptoExportKey",
    "cryptoSign",
    "cryptoVerify",
    "cryptoEncrypt",
    "cryptoDecrypt",
    "cryptoDerive",
    "cryptoCheckPrime",
    "cryptoGeneratePrime",
    "cryptoScrypt",
    "cryptoEcdhPublic",
    "cryptoEcdhCompute",
    "cryptoEcdhConvert",
    "cryptoDhParams",
    "cryptoDhGenerate",
    "cryptoDhCompute",
    "cryptoRsaLegacyPrivateEncrypt",
    "cryptoRsaLegacyPublicDecrypt",
];

pub(super) fn export_host_functions<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
) -> rquickjs::Result<()> {
    let export = |name: &str, function: Function<'js>| exports.export(name, function);
    export("digest", Function::new(ctx.clone(), digest)?)?;
    export(
        "cryptoCreateDigest",
        Function::new(ctx.clone(), digest::create)?,
    )?;
    export("cryptoHmac", Function::new(ctx.clone(), hmac)?)?;
    export("cryptoAesGcm", Function::new(ctx.clone(), aes_gcm)?)?;
    export(
        "cryptoCreateCipher",
        Function::new(ctx.clone(), cipher::create)?,
    )?;
    export(
        "cryptoTimingSafeEqual",
        Function::new(ctx.clone(), timing_safe_equal)?,
    )?;
    export("cryptoPbkdf2", Function::new(ctx.clone(), pbkdf2)?)?;
    export("cryptoHkdf", Function::new(ctx.clone(), hkdf)?)?;
    export(
        "cryptoGenerateKey",
        Function::new(ctx.clone(), host_generate_key)?,
    )?;
    export(
        "cryptoImportKey",
        Function::new(ctx.clone(), host_import_key)?,
    )?;
    export(
        "cryptoExportKey",
        Function::new(ctx.clone(), host_export_key)?,
    )?;
    export("cryptoSign", Function::new(ctx.clone(), host_sign)?)?;
    export("cryptoVerify", Function::new(ctx.clone(), host_verify)?)?;
    export("cryptoEncrypt", Function::new(ctx.clone(), host_encrypt)?)?;
    export("cryptoDecrypt", Function::new(ctx.clone(), host_decrypt)?)?;
    export("cryptoDerive", Function::new(ctx.clone(), host_derive)?)?;
    export(
        "cryptoCheckPrime",
        Function::new(ctx.clone(), host_check_prime)?,
    )?;
    export(
        "cryptoGeneratePrime",
        Function::new(ctx.clone(), host_generate_prime)?,
    )?;
    export("cryptoScrypt", Function::new(ctx.clone(), host_scrypt)?)?;
    export(
        "cryptoEcdhPublic",
        Function::new(ctx.clone(), host_ecdh_public)?,
    )?;
    export(
        "cryptoEcdhCompute",
        Function::new(ctx.clone(), host_ecdh_compute)?,
    )?;
    export(
        "cryptoEcdhConvert",
        Function::new(ctx.clone(), host_ecdh_convert)?,
    )?;
    export(
        "cryptoDhParams",
        Function::new(ctx.clone(), host_dh_params)?,
    )?;
    export(
        "cryptoDhGenerate",
        Function::new(ctx.clone(), host_dh_generate)?,
    )?;
    export(
        "cryptoDhCompute",
        Function::new(ctx.clone(), host_dh_compute)?,
    )?;
    export(
        "cryptoRsaLegacyPrivateEncrypt",
        Function::new(ctx.clone(), host_rsa_legacy_private_encrypt)?,
    )?;
    export(
        "cryptoRsaLegacyPublicDecrypt",
        Function::new(ctx.clone(), host_rsa_legacy_public_decrypt)?,
    )?;
    Ok(())
}

/// The process entropy source, infallible for every cryptographic API.
pub(super) fn rng() -> rand_core::UnwrapErr<getrandom::SysRng> {
    rand_core::UnwrapErr(getrandom::SysRng)
}

fn timing_safe_equal<'js>(
    ctx: Ctx<'js>,
    left: TypedArray<'js, u8>,
    right: TypedArray<'js, u8>,
) -> rquickjs::Result<bool> {
    let left = left
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    let right = right
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached buffer"))?;
    if left.len() != right.len() {
        return Err(Exception::throw_type(
            &ctx,
            "Input buffers must have the same byte length",
        ));
    }
    Ok(left.ct_eq(right).into())
}

pub(super) fn digest<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let hash = message_digest(&ctx, &algorithm)?;
    ArrayBuffer::new_copy(ctx, hash.digest(&input))
}

pub(super) fn hmac<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    key: TypedArray<'js, u8>,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let hash = message_digest(&ctx, &algorithm)?;
    let key = bytes(&ctx, key)?;
    let input = bytes(&ctx, input)?;
    let output = hash
        .hmac(&key, &input)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    ArrayBuffer::new_copy(ctx, &output)
}

pub(super) fn pbkdf2<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    password: TypedArray<'js, u8>,
    salt: TypedArray<'js, u8>,
    iterations: u32,
    length: u32,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let hash = message_digest(&ctx, &algorithm)?;
    let password = bytes(&ctx, password)?;
    let salt = bytes(&ctx, salt)?;
    if iterations == 0 {
        return Err(throw_dom_exception(
            &ctx,
            "OperationError",
            "PBKDF2 requires at least one iteration",
        ));
    }
    let mut output = vec![0; length as usize];
    hash.pbkdf2(&password, &salt, iterations, &mut output);
    ArrayBuffer::new_copy(ctx, &output)
}

pub(super) fn hkdf<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    key: TypedArray<'js, u8>,
    salt: TypedArray<'js, u8>,
    info: TypedArray<'js, u8>,
    length: u32,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let hash = message_digest(&ctx, &algorithm)?;
    let key = bytes(&ctx, key)?;
    let salt = bytes(&ctx, salt)?;
    let info = bytes(&ctx, info)?;
    let mut output = vec![0; length as usize];
    // RFC 5869: an absent salt is a hash-length string of zeros.
    let salt = (!salt.is_empty()).then_some(salt.as_slice());
    hash.hkdf(salt, &key, &info, &mut output)
        .map_err(|_| throw_dom_exception(&ctx, "OperationError", "HKDF output is too long"))?;
    ArrayBuffer::new_copy(ctx, &output)
}

pub(super) fn aes_gcm<'js>(
    ctx: Ctx<'js>,
    encrypt: bool,
    key: TypedArray<'js, u8>,
    iv: TypedArray<'js, u8>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let key = bytes(&ctx, key)?;
    let iv = bytes(&ctx, iv)?;
    let input = bytes(&ctx, input)?;
    let tag_length: Option<u32> = options.get("tagLength")?;
    let mode: Option<String> = options.get("mode")?;
    let additional_data: Option<TypedArray<'js, u8>> = options.get("additionalData")?;
    let additional_data = additional_data
        .map(|data| bytes(&ctx, data))
        .transpose()?
        .unwrap_or_default();
    if !aes::valid_key(&key) {
        return Err(throw_dom_exception(
            &ctx,
            "DataError",
            "AES keys must be 128, 192, or 256 bits",
        ));
    }
    let tag_length = tag_length.unwrap_or(128);
    let output = match mode.as_deref().unwrap_or("AES-GCM") {
        "AES-GCM" => aes_gcm_operation(
            &ctx,
            encrypt,
            &key,
            GcmParams {
                iv: &iv,
                additional_data: &additional_data,
                tag_length,
            },
            &input,
        ),
        "AES-CBC" => aes_one_shot(&ctx, Mode::Cbc, encrypt, &key, &iv, &input),
        "AES-CTR" => aes_ctr_operation(&ctx, &key, &iv, &input, tag_length),
        "AES-KW" => aes_kw_operation(&ctx, encrypt, &key, &input),
        _ => Err(throw_dom_exception(
            &ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }?;
    ArrayBuffer::new_copy(ctx, &output)
}

struct GcmParams<'a> {
    iv: &'a [u8],
    additional_data: &'a [u8],
    tag_length: u32,
}

fn aes_gcm_operation(
    ctx: &Ctx<'_>,
    encrypt: bool,
    key: &[u8],
    params: GcmParams<'_>,
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let GcmParams {
        iv,
        additional_data,
        tag_length,
    } = params;
    let tag_bytes = tag_length
        .checked_div(8)
        .map(|length| length as usize)
        .filter(|length| aes::valid_tag_length(*length))
        .ok_or_else(|| throw_dom_exception(ctx, "OperationError", "Invalid AES-GCM tag length"))?;
    let operation = |error: aes::Failure| throw_dom_exception(ctx, "OperationError", error.0);
    let mut stream = aes::stream(Mode::Gcm, encrypt, key, iv, tag_bytes).map_err(operation)?;
    stream.set_aad(additional_data).map_err(operation)?;
    let (input, tag) = if encrypt {
        (input, None)
    } else if input.len() < tag_bytes {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "The ciphertext is too short",
        ));
    } else {
        let (text, tag) = input.split_at(input.len() - tag_bytes);
        (text, Some(tag))
    };
    if let Some(tag) = tag {
        stream.set_tag(tag).map_err(operation)?;
    }
    let mut output = stream.update(input).map_err(operation)?;
    let finished = stream.finish().map_err(operation)?;
    output.extend(finished.output);
    output.extend(finished.tag.unwrap_or_default());
    Ok(output)
}

fn aes_one_shot(
    ctx: &Ctx<'_>,
    mode: Mode,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let operation = |error: aes::Failure| throw_dom_exception(ctx, "OperationError", error.0);
    let mut stream = aes::stream(mode, encrypt, key, iv, 16).map_err(operation)?;
    let mut output = stream.update(input).map_err(operation)?;
    output.extend(stream.finish().map_err(operation)?.output);
    Ok(output)
}

fn aes_ctr_operation(
    ctx: &Ctx<'_>,
    key: &[u8],
    counter: &[u8],
    input: &[u8],
    length: u32,
) -> rquickjs::Result<Vec<u8>> {
    let invalid =
        || throw_dom_exception(ctx, "OperationError", "Invalid AES-CTR counter or length");
    let Ok(counter) = <[u8; 16]>::try_from(counter) else {
        return Err(invalid());
    };
    if !(1..=128).contains(&length) {
        return Err(invalid());
    }
    // The counter wraps within its rightmost `length` bits; a repeated counter
    // block is an error. The stream increments the full block, which is
    // identical until a wrap, so the input is split at the wrap point.
    if length == 128 {
        return aes_one_shot(ctx, Mode::Ctr, true, key, &counter, input);
    }
    let blocks = input.len().div_ceil(16) as u128;
    let capacity = 1u128 << length;
    if blocks > capacity {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "The AES-CTR counter block repeats",
        ));
    }
    let value = u128::from_be_bytes(counter);
    let until_wrap = capacity - (value & (capacity - 1));
    if blocks <= until_wrap {
        return aes_one_shot(ctx, Mode::Ctr, true, key, &counter, input);
    }
    let split = usize::try_from(until_wrap)
        .map_err(|_| Exception::throw_internal(ctx, "AES-CTR split out of range"))?
        * 16;
    let mut output = aes_one_shot(ctx, Mode::Ctr, true, key, &counter, &input[..split])?;
    let wrapped = (value & !(capacity - 1)).to_be_bytes();
    output.extend(aes_one_shot(
        ctx,
        Mode::Ctr,
        true,
        key,
        &wrapped,
        &input[split..],
    )?);
    Ok(output)
}

fn aes_kw_operation(
    ctx: &Ctx<'_>,
    encrypt: bool,
    key: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    if input.len() < if encrypt { 16 } else { 24 } || !input.len().is_multiple_of(8) {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "AES-KW data must be at least 16 bytes and a multiple of 8",
        ));
    }
    let output = if encrypt {
        aes::wrap_key(key, input)
    } else {
        aes::unwrap_key(key, input)
    };
    output.map_err(|error| throw_dom_exception(ctx, "OperationError", error.0))
}

fn host_generate_key<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let output = generate_key(&ctx, &options)?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_import_key<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let input = key_input(&ctx, &input, &options)?;
    let material = parse_key(&ctx, &input, &options)?;
    let output = match material {
        KeyMaterial::Private(key) => bundle_private(&ctx, &key)?,
        KeyMaterial::Public(key) => bundle(&public_der(&ctx, &key)?, None)?,
    };
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_export_key<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let format = required_string(&ctx, &options, "format")?;
    let parse_format = option_string(&options, "keyFormat")?.unwrap_or_else(|| format.clone());
    let input = key_input(&ctx, &input, &options)?;
    let material = parse_key_with_format(&ctx, &input, &options, &parse_format)?;
    let output = match format.as_str() {
        "spki" => public_der(&ctx, &material.public())?,
        "pkcs8" => private_der(&ctx, &material)?,
        "raw" => material.public().to_raw().ok_or_else(|| {
            throw_dom_exception(
                &ctx,
                "NotSupportedError",
                "Raw export is not supported for this algorithm",
            )
        })?,
        "jwk" => {
            let jwk = match &material {
                KeyMaterial::Private(key) => {
                    key.to_jwk().map_err(|error| operation(&ctx, &error))?
                }
                KeyMaterial::Public(key) => key.to_jwk(),
            };
            serde_json::to_vec(&jwk)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
        }
        _ => {
            return Err(throw_dom_exception(
                &ctx,
                "NotSupportedError",
                "The requested key format is not supported",
            ));
        }
    };
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_sign<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let output = sign_key(&ctx, &input, &options)?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_verify<'js>(
    ctx: Ctx<'js>,
    signature: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<bool> {
    let signature = bytes(&ctx, signature)?;
    verify_key(&ctx, &signature, &options)
}

fn host_encrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let (material, hash, label) = oaep_operands(&ctx, &options)?;
    let output = rsa_public(&ctx, &material.public()).and_then(|key| {
        rsa::encrypt_oaep(key, hash, &label, &input).map_err(|error| operation(&ctx, &error))
    })?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_decrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let (material, hash, label) = oaep_operands(&ctx, &options)?;
    let key = rsa_private(&ctx, &material)?;
    let output =
        rsa::decrypt_oaep(key, hash, &label, &input).map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn oaep_operands<'js>(
    ctx: &Ctx<'js>,
    options: &Object<'js>,
) -> rquickjs::Result<(KeyMaterial, Hash, Vec<u8>)> {
    if required_string(ctx, options, "kind")? != "rsa-oaep" {
        return Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        ));
    }
    let key_bytes = required_bytes(ctx, options, "key")?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let hash = message_digest(ctx, &required_string(ctx, options, "hash")?)?;
    let label = optional_bytes(ctx, options, "label")?;
    Ok((material, hash, label))
}

fn host_derive<'js>(ctx: Ctx<'js>, options: Object<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let key_bytes = required_bytes(&ctx, &options, "key")?;
    let material = parse_key(&ctx, &key_bytes, &options)?;
    let peer = required_bytes(&ctx, &options, "peer")?;
    let peer = PublicKey::from_spki(&peer).map_err(|error| operation(&ctx, &error))?;
    let KeyMaterial::Private(private) = material else {
        return Err(private_required(&ctx));
    };
    let output = match (&private, &peer) {
        (PrivateKey::Ec(key), PublicKey::Ec(peer)) => {
            curves::agree(key, peer).map_err(|error| operation(&ctx, &error))?
        }
        (PrivateKey::X25519(key), PublicKey::X25519(peer)) => curves::x25519_agree(key, peer),
        _ => {
            return Err(throw_dom_exception(
                &ctx,
                "OperationError",
                "The keys do not share an algorithm",
            ));
        }
    };
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_check_prime<'js>(
    ctx: Ctx<'js>,
    candidate: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<bool> {
    let candidate = bytes(&ctx, candidate)?;
    let checks: Option<u32> = options.get("checks")?;
    if checks.is_some_and(|checks| i32::try_from(checks).is_err()) {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"checks\" is out of range",
        ));
    }
    Ok(bignum::check_prime(&candidate))
}

fn host_generate_prime<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let bits = required_u32(&ctx, &options, "bits")?;
    if bits < 2 || i32::try_from(bits).is_err() {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"bits\" is out of range",
        ));
    }
    let safe: Option<bool> = options.get("safe")?;
    let add = option_bytes(&ctx, &options, "add")?;
    let rem = option_bytes(&ctx, &options, "rem")?;
    let output =
        bignum::generate_prime(bits, safe.unwrap_or(false), add.as_deref(), rem.as_deref())
            .map_err(|error| throw_dom_exception(&ctx, "OperationError", error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_scrypt<'js>(
    ctx: Ctx<'js>,
    password: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let password = bytes(&ctx, password)?;
    let salt = required_bytes(&ctx, &options, "salt")?;
    let n: Option<u64> = options.get("n")?;
    let r: Option<u64> = options.get("r")?;
    let p: Option<u64> = options.get("p")?;
    let maxmem: Option<u64> = options.get("maxmem")?;
    let n = n.unwrap_or(16_384);
    let r = r.unwrap_or(8);
    let p = p.unwrap_or(1);
    let maxmem = maxmem.unwrap_or(32 * 1024 * 1024);
    let length = required_u32(&ctx, &options, "keyLength")? as usize;
    let invalid = |message: &str| Exception::throw_range(&ctx, message);
    if !n.is_power_of_two() || n < 2 {
        return Err(invalid("N must be a power of two greater than one"));
    }
    if r.checked_mul(128)
        .and_then(|value| value.checked_mul(n.saturating_add(p)))
        .is_none_or(|value| value > maxmem)
    {
        return Err(invalid("memory limit exceeded"));
    }
    let log_n = u8::try_from(n.trailing_zeros()).map_err(|_| invalid("N is too large"))?;
    let (r, p) = (
        u32::try_from(r).map_err(|_| invalid("r is too large"))?,
        u32::try_from(p).map_err(|_| invalid("p is too large"))?,
    );
    let params = scrypt::Params::new(log_n, r, p).map_err(|error| invalid(&error.to_string()))?;
    let mut output = vec![0; length];
    if length > 0 {
        scrypt::scrypt(&password, &salt, &params, &mut output)
            .map_err(|error| invalid(&error.to_string()))?;
    }
    ArrayBuffer::new_copy(ctx, &output)
}

fn node_curve(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Curve> {
    let name = required_string(ctx, options, "curve")?;
    Curve::parse_node(&name).ok_or_else(|| {
        throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested elliptic curve is not supported",
        )
    })
}

fn point_format(ctx: &Ctx<'_>, format: &str) -> rquickjs::Result<bool> {
    match format {
        "compressed" => Ok(true),
        "uncompressed" => Ok(false),
        _ => Err(Exception::throw_type(
            ctx,
            "The point format must be \"compressed\" or \"uncompressed\"",
        )),
    }
}

fn host_ecdh_public<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let curve = node_curve(&ctx, &options)?;
    let private = required_bytes(&ctx, &options, "private")?;
    let key =
        curves::secret_from_scalar(curve, &private).map_err(|error| operation(&ctx, &error))?;
    let format = option_string(&options, "format")?.unwrap_or_else(|| "uncompressed".to_owned());
    let compressed = point_format(&ctx, &format)?;
    ArrayBuffer::new_copy(
        ctx,
        curves::encode_point(&curves::public_of(&key), compressed),
    )
}

fn host_ecdh_compute<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let curve = node_curve(&ctx, &options)?;
    let private = required_bytes(&ctx, &options, "private")?;
    let peer = required_bytes(&ctx, &options, "peer")?;
    let key =
        curves::secret_from_scalar(curve, &private).map_err(|error| operation(&ctx, &error))?;
    let peer = curves::decode_point(curve, &peer).map_err(|error| operation(&ctx, &error))?;
    let output = curves::agree(&key, &peer).map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_ecdh_convert<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let curve = node_curve(&ctx, &options)?;
    let point = curves::decode_point(curve, &input).map_err(|error| operation(&ctx, &error))?;
    let compressed = point_format(&ctx, &required_string(&ctx, &options, "format")?)?;
    ArrayBuffer::new_copy(ctx, curves::encode_point(&point, compressed))
}

fn host_dh_params<'js>(ctx: Ctx<'js>, options: Object<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let bits = required_u32(&ctx, &options, "bits")?;
    let generator: Option<u32> = options.get("generator")?;
    let (prime, generator) = bignum::dh_parameters(bits, generator.unwrap_or(2))
        .map_err(|error| throw_dom_exception(&ctx, "OperationError", error))?;
    ArrayBuffer::new_copy(ctx, &bundle(&prime, Some(&generator))?)
}

fn host_dh_generate<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let prime = required_bytes(&ctx, &options, "prime")?;
    let generator = required_bytes(&ctx, &options, "generator")?;
    let failed = |error| throw_dom_exception(&ctx, "OperationError", error);
    let private = match option_bytes(&ctx, &options, "private")? {
        Some(private) => private,
        None => bignum::random_below(&prime).map_err(failed)?,
    };
    let public = bignum::mod_pow(&generator, &private, &prime).map_err(failed)?;
    ArrayBuffer::new_copy(ctx, &bundle(&public, Some(&private))?)
}

fn host_dh_compute<'js>(ctx: Ctx<'js>, options: Object<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let prime = required_bytes(&ctx, &options, "prime")?;
    let private = required_bytes(&ctx, &options, "private")?;
    let peer = required_bytes(&ctx, &options, "peer")?;
    let secret = bignum::mod_pow(&peer, &private, &prime)
        .map_err(|error| throw_dom_exception(&ctx, "OperationError", error))?;
    ArrayBuffer::new_copy(ctx, &secret)
}

fn host_rsa_legacy_private_encrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let key_bytes = required_bytes(&ctx, &options, "key")?;
    let material = parse_key(&ctx, &key_bytes, &options)?;
    let key = rsa_private(&ctx, &material)?;
    let padding = rsa_padding(&ctx, &options)?;
    let output =
        rsa::private_encrypt(key, padding, &input).map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_rsa_legacy_public_decrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let key_bytes = required_bytes(&ctx, &options, "key")?;
    let material = parse_key(&ctx, &key_bytes, &options)?;
    let padding = rsa_padding(&ctx, &options)?;
    let output = rsa_public(&ctx, &material.public()).and_then(|key| {
        rsa::public_decrypt(key, padding, &input).map_err(|error| operation(&ctx, &error))
    })?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn rsa_padding(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<rsa::Padding> {
    let padding: Option<u32> = options.get("padding")?;
    rsa::Padding::parse(padding).ok_or_else(|| {
        throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested RSA padding mode is not supported",
        )
    })
}

fn generate_key(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let kind = required_string(ctx, options, "kind")?;
    let key = match kind.as_str() {
        "rsa" | "rsa-pss" | "rsa-oaep" => {
            let modulus_length = required_u32(ctx, options, "modulusLength")?;
            let exponent = required_bytes(ctx, options, "publicExponent")?;
            PrivateKey::Rsa(
                rsa::generate(modulus_length as usize, &exponent)
                    .map_err(|error| operation(ctx, &error))?,
            )
        }
        "ec" => PrivateKey::Ec(curves::generate(web_curve(ctx, options)?)),
        "ed25519" => PrivateKey::Ed25519(ed25519_dalek::SigningKey::generate(&mut rng())),
        "x25519" => PrivateKey::X25519(x25519_dalek::StaticSecret::random_from_rng(&mut rng())),
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "NotSupportedError",
                "The requested cryptographic algorithm is not supported",
            ));
        }
    };
    bundle_private(ctx, &key)
}

fn web_curve(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Curve> {
    let name = required_string(ctx, options, "curve")?;
    Curve::parse(&name).ok_or_else(|| {
        throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested elliptic curve is not supported",
        )
    })
}

fn parse_key(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<KeyMaterial> {
    let format = required_string(ctx, options, "format")?;
    parse_key_with_format(ctx, input, options, &format)
}

fn parse_key_with_format(
    ctx: &Ctx<'_>,
    input: &[u8],
    options: &Object<'_>,
    format: &str,
) -> rquickjs::Result<KeyMaterial> {
    let kind_name = required_string(ctx, options, "kind")?;
    let kind = Kind::parse(&kind_name);
    let data_error = |error: KeyError| match error {
        KeyError::Data(message) => throw_dom_exception(ctx, "DataError", &message),
        KeyError::Operation(message) => throw_dom_exception(ctx, "OperationError", &message),
    };
    let material = match format {
        "spki" => KeyMaterial::Public(PublicKey::from_spki(input).map_err(data_error)?),
        "pkcs8" => KeyMaterial::Private(PrivateKey::from_pkcs8(input).map_err(data_error)?),
        "raw" => {
            let Some(kind) = kind.filter(|kind| *kind != Kind::Rsa) else {
                return Err(throw_dom_exception(
                    ctx,
                    "NotSupportedError",
                    "Raw import is not supported for this algorithm",
                ));
            };
            let curve = option_string(options, "curve")?.and_then(|name| Curve::parse(&name));
            KeyMaterial::Public(PublicKey::from_raw(kind, curve, input).map_err(data_error)?)
        }
        "jwk" => {
            let text = required_string(ctx, options, "jwk")?;
            let jwk: JsonValue = serde_json::from_str(&text)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
            let kind = kind.ok_or_else(|| {
                throw_dom_exception(
                    ctx,
                    "DataError",
                    "The JWK does not match the requested algorithm",
                )
            })?;
            keys::from_jwk(kind, &jwk).map_err(data_error)?
        }
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "NotSupportedError",
                "The requested key format is not supported",
            ));
        }
    };
    if kind != Some(material.kind()) {
        return Err(throw_dom_exception(
            ctx,
            "DataError",
            "The key does not match the requested algorithm",
        ));
    }
    Ok(material)
}

fn bundle_private(ctx: &Ctx<'_>, key: &PrivateKey) -> rquickjs::Result<Vec<u8>> {
    let public = public_der(ctx, &key.public())?;
    let private = key.to_pkcs8().map_err(|error| operation(ctx, &error))?;
    bundle(&public, Some(&private))
}

fn bundle(public: &[u8], private: Option<&[u8]>) -> rquickjs::Result<Vec<u8>> {
    let private_len = private.map_or(0, <[u8]>::len);
    let public_len =
        u32::try_from(public.len()).map_err(|_| rquickjs::Error::new_from_js("key", "key"))?;
    let private_len =
        u32::try_from(private_len).map_err(|_| rquickjs::Error::new_from_js("key", "key"))?;
    let mut output = Vec::with_capacity(9 + public.len() + private_len as usize);
    output.push(1);
    output.extend_from_slice(&public_len.to_be_bytes());
    output.extend_from_slice(&private_len.to_be_bytes());
    output.extend_from_slice(public);
    if let Some(private) = private {
        output.extend_from_slice(private);
    }
    Ok(output)
}

fn public_der(ctx: &Ctx<'_>, key: &PublicKey) -> rquickjs::Result<Vec<u8>> {
    key.to_spki().map_err(|error| operation(ctx, &error))
}

fn private_der(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    match material {
        KeyMaterial::Private(key) => key.to_pkcs8().map_err(|error| operation(ctx, &error)),
        KeyMaterial::Public(_) => Err(throw_dom_exception(
            ctx,
            "InvalidAccessError",
            "The key is not private",
        )),
    }
}

fn rsa_private<'a>(
    ctx: &Ctx<'_>,
    material: &'a KeyMaterial,
) -> rquickjs::Result<&'a ::rsa::RsaPrivateKey> {
    match material {
        KeyMaterial::Private(PrivateKey::Rsa(key)) => Ok(key),
        KeyMaterial::Private(_) => Err(throw_dom_exception(
            ctx,
            "OperationError",
            "An RSA key is required",
        )),
        KeyMaterial::Public(_) => Err(private_required(ctx)),
    }
}

fn rsa_public<'a>(ctx: &Ctx<'_>, key: &'a PublicKey) -> rquickjs::Result<&'a ::rsa::RsaPublicKey> {
    match key {
        PublicKey::Rsa(key) => Ok(key),
        _ => Err(throw_dom_exception(
            ctx,
            "OperationError",
            "An RSA key is required",
        )),
    }
}

fn private_required(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_dom_exception(ctx, "InvalidAccessError", "A private key is required")
}

fn sign_key(ctx: &Ctx<'_>, message: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let kind = required_string(ctx, options, "kind")?;
    let key_bytes = required_bytes(ctx, options, "key")?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let hash = || message_digest(ctx, &required_string(ctx, options, "hash")?);
    let KeyMaterial::Private(key) = &material else {
        return Err(private_required(ctx));
    };
    let failed = |error: KeyError| operation(ctx, &error);
    match (kind.as_str(), key) {
        ("rsa", PrivateKey::Rsa(key)) => rsa::sign_pkcs1(key, hash()?, message).map_err(failed),
        ("rsa-pss", PrivateKey::Rsa(key)) => {
            let salt_length = required_u32(ctx, options, "saltLength")? as usize;
            rsa::sign_pss(key, hash()?, salt_length, message).map_err(failed)
        }
        ("ec" | "ecdsa", PrivateKey::Ec(key)) => {
            curves::sign(key, &hash()?.digest(message)).map_err(failed)
        }
        ("ed25519", PrivateKey::Ed25519(key)) => Ok(curves::ed25519_sign(key, message)),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }
}

fn verify_key(ctx: &Ctx<'_>, signature: &[u8], options: &Object<'_>) -> rquickjs::Result<bool> {
    let kind = required_string(ctx, options, "kind")?;
    let message = required_bytes(ctx, options, "message")?;
    let key_bytes = required_bytes(ctx, options, "key")?;
    let key = parse_key(ctx, &key_bytes, options)?.public();
    let hash = || message_digest(ctx, &required_string(ctx, options, "hash")?);
    match (kind.as_str(), &key) {
        ("rsa", PublicKey::Rsa(key)) => Ok(rsa::verify_pkcs1(key, hash()?, &message, signature)),
        ("rsa-pss", PublicKey::Rsa(key)) => {
            let salt_length = required_u32(ctx, options, "saltLength")? as usize;
            Ok(rsa::verify_pss(
                key,
                hash()?,
                salt_length,
                &message,
                signature,
            ))
        }
        ("ec" | "ecdsa", PublicKey::Ec(key)) => {
            Ok(curves::verify(key, &hash()?.digest(&message), signature))
        }
        ("ed25519", PublicKey::Ed25519(key)) => {
            Ok(curves::ed25519_verify(key, &message, signature))
        }
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }
}

fn operation(ctx: &Ctx<'_>, error: &KeyError) -> rquickjs::Error {
    throw_dom_exception(ctx, "OperationError", error.message())
}

fn message_digest(ctx: &Ctx<'_>, algorithm: &str) -> rquickjs::Result<Hash> {
    Hash::parse(algorithm).ok_or_else(|| {
        throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )
    })
}

fn required_string(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<String> {
    option_string(options, name)?
        .ok_or_else(|| Exception::throw_type(ctx, &format!("{name} is required")))
}

fn option_string(options: &Object<'_>, name: &str) -> rquickjs::Result<Option<String>> {
    options.get(name)
}

fn required_u32(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<u32> {
    let value: Option<u32> = options.get(name)?;
    value.ok_or_else(|| Exception::throw_type(ctx, &format!("{name} is required")))
}

fn required_bytes(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<Vec<u8>> {
    option_bytes(ctx, options, name)?
        .ok_or_else(|| Exception::throw_type(ctx, &format!("{name} is required")))
}

fn key_input(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    Ok(option_bytes(ctx, options, "key")?.unwrap_or_else(|| input.to_owned()))
}

fn optional_bytes(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<Vec<u8>> {
    Ok(option_bytes(ctx, options, name)?.unwrap_or_default())
}

fn option_bytes<'js>(
    ctx: &Ctx<'_>,
    options: &Object<'js>,
    name: &str,
) -> rquickjs::Result<Option<Vec<u8>>> {
    let value: Option<TypedArray<'js, u8>> = options.get(name)?;
    value
        .map(|value| {
            value
                .as_bytes()
                .map(ToOwned::to_owned)
                .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))
        })
        .transpose()
}

fn bytes<'js>(ctx: &Ctx<'js>, input: TypedArray<'js, u8>) -> rquickjs::Result<Vec<u8>> {
    input
        .as_bytes()
        .map(ToOwned::to_owned)
        .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))
}

fn throw_dom_exception(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let constructor: Constructor = match ctx.globals().get("DOMException") {
        Ok(constructor) => constructor,
        Err(error) => return error,
    };
    let exception = match constructor.construct::<_, Object>((message, name)) {
        Ok(exception) => exception,
        Err(error) => return error,
    };
    ctx.throw(exception.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn one_shot_hmac_accepts_an_empty_key() -> rquickjs::Result<()> {
        let runtime = rquickjs::Runtime::new()?;
        let context = rquickjs::Context::full(&runtime)?;
        context.with(|ctx| {
            let key = TypedArray::new(ctx.clone(), Vec::<u8>::new())?;
            let input = TypedArray::new(ctx.clone(), b"abc".to_vec())?;
            let output = hmac(ctx.clone(), "SHA-256".into(), key, input)?;
            let bytes = output
                .as_bytes()
                .ok_or_else(|| Exception::throw_message(&ctx, "detached ArrayBuffer"))?;
            // Pinned Workerd createHmac('sha256', '').update('abc').digest('base64').
            assert_eq!(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                "/XrbFSwF74Dcz1Ch+kwF1aPsbalVdfwxKufF0JGDY1E="
            );
            Ok(())
        })
    }
}
