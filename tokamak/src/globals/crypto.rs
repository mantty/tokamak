#![allow(clippy::needless_pass_by_value)]

mod aes;
mod bignum;
mod cipher;
mod curves;
mod digest;
mod hash;
mod keys;
mod rsa;

use rquickjs::{ArrayBuffer, Constructor, Ctx, Exception, Object, TypedArray};
use serde_json::Value as JsonValue;
use subtle::ConstantTimeEq;

use self::aes::{Mode, Stream};
use self::curves::Curve;
use self::hash::Hash;
use self::keys::{KeyError, KeyMaterial, Kind, PrivateKey, PublicKey};

super::host_functions! {
    pub(super),
    "digest" => digest,
    "cryptoCreateDigest" => digest::create,
    "cryptoHmac" => hmac,
    "cryptoAesGcm" => aes_gcm,
    "cryptoCreateCipher" => cipher::create,
    "cryptoTimingSafeEqual" => timing_safe_equal,
    "cryptoPbkdf2" => pbkdf2,
    "cryptoHkdf" => hkdf,
    "cryptoGenerateKey" => host_generate_key,
    "cryptoImportKey" => host_import_key,
    "cryptoExportKey" => host_export_key,
    "cryptoSign" => host_sign,
    "cryptoVerify" => host_verify,
    "cryptoEncrypt" => host_encrypt,
    "cryptoDecrypt" => host_decrypt,
    "cryptoDerive" => host_derive,
    "cryptoCheckPrime" => host_check_prime,
    "cryptoGeneratePrime" => host_generate_prime,
    "cryptoScrypt" => host_scrypt,
    "cryptoEcdhPublic" => host_ecdh_public,
    "cryptoEcdhCompute" => host_ecdh_compute,
    "cryptoEcdhConvert" => host_ecdh_convert,
    "cryptoDhParams" => host_dh_params,
    "cryptoDhGenerate" => host_dh_generate,
    "cryptoDhCompute" => host_dh_compute,
    "cryptoRsaLegacyPrivateEncrypt" => host_rsa_legacy_private_encrypt,
    "cryptoRsaLegacyPublicDecrypt" => host_rsa_legacy_public_decrypt,
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
    let left = byte_slice(&ctx, &left)?;
    let right = byte_slice(&ctx, &right)?;
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
        return Err(operation_error(
            &ctx,
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
        .map_err(|_| operation_error(&ctx, "HKDF output is too long"))?;
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
    let tag_length = options.get::<_, Option<u32>>("tagLength")?.unwrap_or(128);
    let mode: Option<String> = options.get("mode")?;
    let additional_data = optional_bytes(&ctx, &options, "additionalData")?;
    if !aes::valid_key(&key) {
        return Err(data_error(&ctx, "AES keys must be 128, 192, or 256 bits"));
    }
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
        _ => Err(unsupported_algorithm(&ctx)),
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
    let tag_bytes = usize::try_from(tag_length / 8)
        .ok()
        .filter(|length| aes::valid_tag_length(*length))
        .ok_or_else(|| operation_error(ctx, "Invalid AES-GCM tag length"))?;
    let failed = |error: aes::Failure| operation_error(ctx, error.0);
    let mut stream = aes::stream(Mode::Gcm, encrypt, key, iv, tag_bytes).map_err(failed)?;
    stream.set_aad(additional_data).map_err(failed)?;
    let text = if encrypt {
        input
    } else {
        let Some(split) = input.len().checked_sub(tag_bytes) else {
            return Err(operation_error(ctx, "The ciphertext is too short"));
        };
        let (text, tag) = input.split_at(split);
        stream.set_tag(tag).map_err(failed)?;
        text
    };
    aes_output(ctx, stream, text)
}

fn aes_one_shot(
    ctx: &Ctx<'_>,
    mode: Mode,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let stream =
        aes::stream(mode, encrypt, key, iv, 16).map_err(|error| operation_error(ctx, error.0))?;
    aes_output(ctx, stream, input)
}

/// Everything `stream` produces for `input`, followed by any tag.
fn aes_output(
    ctx: &Ctx<'_>,
    mut stream: Box<dyn Stream>,
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let failed = |error: aes::Failure| operation_error(ctx, error.0);
    let mut output = stream.update(input).map_err(failed)?;
    let finished = stream.finish().map_err(failed)?;
    output.extend(finished.output);
    output.extend(finished.tag.unwrap_or_default());
    Ok(output)
}

fn aes_ctr_operation(
    ctx: &Ctx<'_>,
    key: &[u8],
    counter: &[u8],
    input: &[u8],
    length: u32,
) -> rquickjs::Result<Vec<u8>> {
    let invalid = || operation_error(ctx, "Invalid AES-CTR counter or length");
    let Ok(counter) = <[u8; 16]>::try_from(counter) else {
        return Err(invalid());
    };
    if !(1..=128).contains(&length) {
        return Err(invalid());
    }
    let ctr =
        |counter: &[u8], input: &[u8]| aes_one_shot(ctx, Mode::Ctr, true, key, counter, input);
    // The counter wraps within its rightmost `length` bits; a repeated counter
    // block is an error. The stream increments the full block, which is
    // identical until a wrap, so the input is split at the wrap point.
    if length == 128 {
        return ctr(&counter, input);
    }
    let blocks = input.len().div_ceil(16) as u128;
    let capacity = 1u128 << length;
    if blocks > capacity {
        return Err(operation_error(ctx, "The AES-CTR counter block repeats"));
    }
    let value = u128::from_be_bytes(counter);
    let until_wrap = capacity - (value & (capacity - 1));
    if blocks <= until_wrap {
        return ctr(&counter, input);
    }
    let split = usize::try_from(until_wrap)
        .map_err(|_| Exception::throw_internal(ctx, "AES-CTR split out of range"))?
        * 16;
    let mut output = ctr(&counter, &input[..split])?;
    let wrapped = (value & !(capacity - 1)).to_be_bytes();
    output.extend(ctr(&wrapped, &input[split..])?);
    Ok(output)
}

fn aes_kw_operation(
    ctx: &Ctx<'_>,
    encrypt: bool,
    key: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    if input.len() < if encrypt { 16 } else { 24 } || !input.len().is_multiple_of(8) {
        return Err(operation_error(
            ctx,
            "AES-KW data must be at least 16 bytes and a multiple of 8",
        ));
    }
    let output = if encrypt {
        aes::wrap_key(key, input)
    } else {
        aes::unwrap_key(key, input)
    };
    output.map_err(|error| operation_error(ctx, error.0))
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
    let key_format = option_string(&options, "keyFormat")?;
    let parse_format = key_format.as_deref().unwrap_or(format.as_str());
    let input = key_input(&ctx, &input, &options)?;
    let material = parse_key_with_format(&ctx, &input, &options, parse_format)?;
    let output = match format.as_str() {
        "spki" => public_der(&ctx, &material.public())?,
        "pkcs8" => private_der(&ctx, &material)?,
        "raw" => material.public().to_raw().ok_or_else(|| {
            not_supported_error(&ctx, "Raw export is not supported for this algorithm")
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
        _ => return Err(unsupported_format(&ctx)),
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
        return Err(unsupported_algorithm(ctx));
    }
    let material = required_key(ctx, options)?;
    let hash = message_digest(ctx, &required_string(ctx, options, "hash")?)?;
    let label = optional_bytes(ctx, options, "label")?;
    Ok((material, hash, label))
}

fn host_derive<'js>(ctx: Ctx<'js>, options: Object<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let material = required_key(&ctx, &options)?;
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
        _ => return Err(operation_error(&ctx, "The keys do not share an algorithm")),
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
    let safe = options.get::<_, Option<bool>>("safe")?.unwrap_or(false);
    let add = option_bytes(&ctx, &options, "add")?;
    let rem = option_bytes(&ctx, &options, "rem")?;
    let output = bignum::generate_prime(bits, safe, add.as_deref(), rem.as_deref())
        .map_err(|error| operation_error(&ctx, error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn host_scrypt<'js>(
    ctx: Ctx<'js>,
    password: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let password = bytes(&ctx, password)?;
    let salt = required_bytes(&ctx, &options, "salt")?;
    let n = options.get::<_, Option<u64>>("n")?.unwrap_or(16_384);
    let r = options.get::<_, Option<u64>>("r")?.unwrap_or(8);
    let p = options.get::<_, Option<u64>>("p")?.unwrap_or(1);
    let maxmem = options
        .get::<_, Option<u64>>("maxmem")?
        .unwrap_or(32 * 1024 * 1024);
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
    let r = u32::try_from(r).map_err(|_| invalid("r is too large"))?;
    let p = u32::try_from(p).map_err(|_| invalid("p is too large"))?;
    let params = scrypt::Params::new(log_n, r, p).map_err(|error| invalid(&error.to_string()))?;
    let mut output = vec![0; length];
    if length > 0 {
        scrypt::scrypt(&password, &salt, &params, &mut output)
            .map_err(|error| invalid(&error.to_string()))?;
    }
    ArrayBuffer::new_copy(ctx, &output)
}

/// The `curve` option, read by `parse` as `WebCrypto` or node:crypto spells it.
fn required_curve(
    ctx: &Ctx<'_>,
    options: &Object<'_>,
    parse: fn(&str) -> Option<Curve>,
) -> rquickjs::Result<Curve> {
    let name = required_string(ctx, options, "curve")?;
    parse(&name)
        .ok_or_else(|| not_supported_error(ctx, "The requested elliptic curve is not supported"))
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
    let curve = required_curve(&ctx, &options, Curve::parse_node)?;
    let private = required_bytes(&ctx, &options, "private")?;
    let key =
        curves::secret_from_scalar(curve, &private).map_err(|error| operation(&ctx, &error))?;
    let format = option_string(&options, "format")?;
    let compressed = point_format(&ctx, format.as_deref().unwrap_or("uncompressed"))?;
    ArrayBuffer::new_copy(
        ctx,
        curves::encode_point(&curves::public_of(&key), compressed),
    )
}

fn host_ecdh_compute<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let curve = required_curve(&ctx, &options, Curve::parse_node)?;
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
    let curve = required_curve(&ctx, &options, Curve::parse_node)?;
    let point = curves::decode_point(curve, &input).map_err(|error| operation(&ctx, &error))?;
    let compressed = point_format(&ctx, &required_string(&ctx, &options, "format")?)?;
    ArrayBuffer::new_copy(ctx, curves::encode_point(&point, compressed))
}

fn host_dh_params<'js>(ctx: Ctx<'js>, options: Object<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let bits = required_u32(&ctx, &options, "bits")?;
    let generator: Option<u32> = options.get("generator")?;
    let (prime, generator) = bignum::dh_parameters(bits, generator.unwrap_or(2))
        .map_err(|error| operation_error(&ctx, error))?;
    ArrayBuffer::new_copy(ctx, &bundle(&prime, Some(&generator))?)
}

fn host_dh_generate<'js>(
    ctx: Ctx<'js>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let prime = required_bytes(&ctx, &options, "prime")?;
    let generator = required_bytes(&ctx, &options, "generator")?;
    let failed = |error| operation_error(&ctx, error);
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
    let secret =
        bignum::mod_pow(&peer, &private, &prime).map_err(|error| operation_error(&ctx, error))?;
    ArrayBuffer::new_copy(ctx, &secret)
}

fn host_rsa_legacy_private_encrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Object<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let material = required_key(&ctx, &options)?;
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
    let material = required_key(&ctx, &options)?;
    let padding = rsa_padding(&ctx, &options)?;
    let output = rsa_public(&ctx, &material.public()).and_then(|key| {
        rsa::public_decrypt(key, padding, &input).map_err(|error| operation(&ctx, &error))
    })?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn rsa_padding(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<rsa::Padding> {
    let padding: Option<u32> = options.get("padding")?;
    rsa::Padding::parse(padding)
        .ok_or_else(|| not_supported_error(ctx, "The requested RSA padding mode is not supported"))
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
        "ec" => PrivateKey::Ec(curves::generate(required_curve(
            ctx,
            options,
            Curve::parse,
        )?)),
        "ed25519" => PrivateKey::Ed25519(ed25519_dalek::SigningKey::generate(&mut rng())),
        "x25519" => PrivateKey::X25519(x25519_dalek::StaticSecret::random_from_rng(&mut rng())),
        _ => return Err(unsupported_algorithm(ctx)),
    };
    bundle_private(ctx, &key)
}

/// The key in the `key` option, parsed as the options describe it.
fn required_key(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<KeyMaterial> {
    let key_bytes = required_bytes(ctx, options, "key")?;
    parse_key(ctx, &key_bytes, options)
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
    let key_error = |error: KeyError| match error {
        KeyError::Data(message) => data_error(ctx, &message),
        KeyError::Operation(message) => operation_error(ctx, &message),
    };
    let material = match format {
        "spki" => KeyMaterial::Public(PublicKey::from_spki(input).map_err(key_error)?),
        "pkcs8" => KeyMaterial::Private(PrivateKey::from_pkcs8(input).map_err(key_error)?),
        "raw" => {
            let Some(kind) = kind.filter(|kind| *kind != Kind::Rsa) else {
                return Err(not_supported_error(
                    ctx,
                    "Raw import is not supported for this algorithm",
                ));
            };
            let curve = option_string(options, "curve")?.and_then(|name| Curve::parse(&name));
            KeyMaterial::Public(PublicKey::from_raw(kind, curve, input).map_err(key_error)?)
        }
        "jwk" => {
            let text = required_string(ctx, options, "jwk")?;
            let jwk: JsonValue =
                serde_json::from_str(&text).map_err(|error| data_error(ctx, &error.to_string()))?;
            let kind = kind
                .ok_or_else(|| data_error(ctx, "The JWK does not match the requested algorithm"))?;
            keys::from_jwk(kind, &jwk).map_err(key_error)?
        }
        _ => return Err(unsupported_format(ctx)),
    };
    if kind != Some(material.kind()) {
        return Err(data_error(
            ctx,
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
    let private = private.unwrap_or_default();
    let length = |part: &[u8]| {
        u32::try_from(part.len()).map_err(|_| rquickjs::Error::new_from_js("key", "key"))
    };
    let public_len = length(public)?;
    let private_len = length(private)?;
    let mut output = Vec::with_capacity(9 + public.len() + private.len());
    output.push(1);
    output.extend_from_slice(&public_len.to_be_bytes());
    output.extend_from_slice(&private_len.to_be_bytes());
    output.extend_from_slice(public);
    output.extend_from_slice(private);
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
        KeyMaterial::Private(_) => Err(operation_error(ctx, "An RSA key is required")),
        KeyMaterial::Public(_) => Err(private_required(ctx)),
    }
}

fn rsa_public<'a>(ctx: &Ctx<'_>, key: &'a PublicKey) -> rquickjs::Result<&'a ::rsa::RsaPublicKey> {
    match key {
        PublicKey::Rsa(key) => Ok(key),
        _ => Err(operation_error(ctx, "An RSA key is required")),
    }
}

fn private_required(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_dom_exception(ctx, "InvalidAccessError", "A private key is required")
}

fn sign_key(ctx: &Ctx<'_>, message: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let kind = required_string(ctx, options, "kind")?;
    let material = required_key(ctx, options)?;
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
        _ => Err(unsupported_algorithm(ctx)),
    }
}

fn verify_key(ctx: &Ctx<'_>, signature: &[u8], options: &Object<'_>) -> rquickjs::Result<bool> {
    let kind = required_string(ctx, options, "kind")?;
    let message = required_bytes(ctx, options, "message")?;
    let key = required_key(ctx, options)?.public();
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
        _ => Err(unsupported_algorithm(ctx)),
    }
}

fn message_digest(ctx: &Ctx<'_>, algorithm: &str) -> rquickjs::Result<Hash> {
    Hash::parse(algorithm).ok_or_else(|| unsupported_algorithm(ctx))
}

fn required_string(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<String> {
    option_string(options, name)?.ok_or_else(|| missing(ctx, name))
}

fn option_string(options: &Object<'_>, name: &str) -> rquickjs::Result<Option<String>> {
    options.get(name)
}

fn required_u32(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<u32> {
    options
        .get::<_, Option<u32>>(name)?
        .ok_or_else(|| missing(ctx, name))
}

fn required_bytes(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<Vec<u8>> {
    option_bytes(ctx, options, name)?.ok_or_else(|| missing(ctx, name))
}

fn missing(ctx: &Ctx<'_>, name: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, &format!("{name} is required"))
}

fn key_input(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    Ok(option_bytes(ctx, options, "key")?.unwrap_or_else(|| input.to_owned()))
}

fn optional_bytes(ctx: &Ctx<'_>, options: &Object<'_>, name: &str) -> rquickjs::Result<Vec<u8>> {
    Ok(option_bytes(ctx, options, name)?.unwrap_or_default())
}

fn option_bytes(
    ctx: &Ctx<'_>,
    options: &Object<'_>,
    name: &str,
) -> rquickjs::Result<Option<Vec<u8>>> {
    let value: Option<TypedArray<'_, u8>> = options.get(name)?;
    value.map(|value| bytes(ctx, value)).transpose()
}

fn bytes(ctx: &Ctx<'_>, input: TypedArray<'_, u8>) -> rquickjs::Result<Vec<u8>> {
    byte_slice(ctx, &input).map(ToOwned::to_owned)
}

fn byte_slice<'a>(ctx: &Ctx<'_>, input: &'a TypedArray<'_, u8>) -> rquickjs::Result<&'a [u8]> {
    input
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))
}

fn operation(ctx: &Ctx<'_>, error: &KeyError) -> rquickjs::Error {
    operation_error(ctx, error.message())
}

fn operation_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    throw_dom_exception(ctx, "OperationError", message)
}

fn data_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    throw_dom_exception(ctx, "DataError", message)
}

fn not_supported_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    throw_dom_exception(ctx, "NotSupportedError", message)
}

fn unsupported_algorithm(ctx: &Ctx<'_>) -> rquickjs::Error {
    not_supported_error(
        ctx,
        "The requested cryptographic algorithm is not supported",
    )
}

fn unsupported_format(ctx: &Ctx<'_>) -> rquickjs::Error {
    not_supported_error(ctx, "The requested key format is not supported")
}

fn throw_dom_exception(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let exception = ctx
        .globals()
        .get::<_, Constructor>("DOMException")
        .and_then(|constructor| constructor.construct::<_, Object>((message, name)));
    match exception {
        Ok(exception) => ctx.throw(exception.into()),
        Err(error) => error,
    }
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
