#![allow(clippy::needless_pass_by_value)]

mod aes;
mod bignum;
mod cipher;
mod curves;
mod digest;
mod hash;
mod keys;
mod rsa;

use ::rsa::traits::PublicKeyParts;
use rquickjs::{ArrayBuffer, Ctx, Exception, FromJs, IntoJs, TypedArray};
use serde_json::Value as JsonValue;
use subtle::ConstantTimeEq;
use tokamak_macros::{JsType, host_function};

use self::aes::{Mode, Stream};
use self::curves::Curve;
use self::hash::Hash;
use self::keys::{KeyError, KeyMaterial, Kind, PrivateKey, PublicKey};
use super::native::throw_dom_exception;

tokamak_macros::host_functions! {
    pub(super),
    "digest" => digest,
    "cryptoCreateDigest" => digest::create,
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

#[host_function]
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

#[host_function]
pub(super) fn digest<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let hash = message_digest(&ctx, &algorithm)?;
    ArrayBuffer::new_copy(ctx, hash.digest(&input))
}

#[host_function]
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

#[host_function]
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

/// An AES operation's mode, GCM's tag length or CTR's counter length in bits,
/// and the data GCM authenticates with the input.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct AesParams<'js> {
    tag_length: u32,
    mode: String,
    additional_data: Option<TypedArray<'js, u8>>,
}

#[host_function]
pub(super) fn aes_gcm<'js>(
    ctx: Ctx<'js>,
    encrypt: bool,
    key: TypedArray<'js, u8>,
    iv: TypedArray<'js, u8>,
    input: TypedArray<'js, u8>,
    options: AesParams<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let key = bytes(&ctx, key)?;
    let iv = bytes(&ctx, iv)?;
    let input = bytes(&ctx, input)?;
    let AesParams {
        tag_length,
        mode,
        additional_data,
    } = options;
    let additional_data = optional_bytes(&ctx, additional_data)?;
    let output = match mode.as_str() {
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
        .map_err(|_| operation_error(ctx, "Invalid AES-GCM tag length"))?;
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

/// The key `cryptoGenerateKey` makes: its kind, with an RSA key's modulus
/// length and public exponent, or an EC key's curve.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct KeyGeneration<'js> {
    kind: String,
    modulus_length: Option<u32>,
    public_exponent: Option<TypedArray<'js, u8>>,
    curve: Option<String>,
}

#[host_function]
fn host_generate_key<'js>(
    ctx: Ctx<'js>,
    options: KeyGeneration<'js>,
) -> rquickjs::Result<HostKey<'js>> {
    let key = generate_key(&ctx, options)?;
    HostKey::new(&ctx, &KeyMaterial::Private(key))
}

/// A key to import: its bytes in `format`, or the text of a JSON Web Key,
/// which must be of `kind` when one is given. A raw EC key names its curve.
#[derive(FromJs, JsType)]
pub(crate) struct KeyImport<'js> {
    format: String,
    kind: Option<String>,
    curve: Option<String>,
    key: Option<TypedArray<'js, u8>>,
    jwk: Option<String>,
}

#[host_function]
fn host_import_key<'js>(ctx: Ctx<'js>, options: KeyImport<'js>) -> rquickjs::Result<HostKey<'js>> {
    let input = optional_bytes(&ctx, options.key)?;
    let kind = options.kind.as_deref();
    let material = match options.format.as_str() {
        "raw" => raw_key(&ctx, &input, kind, options.curve.as_deref())?,
        "jwk" => jwk_key(&ctx, kind, options.jwk)?,
        format => parse_der(&ctx, &input, format)?,
    };
    HostKey::new(&ctx, &of_kind(&ctx, material, kind)?)
}

/// A key as the JavaScript layers hold it: its kind, its SPKI and, when private,
/// its PKCS#8, with an RSA key's modulus length and exponent or an EC key's curve.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct HostKey<'js> {
    kind: &'static str,
    public: TypedArray<'js, u8>,
    private: Option<TypedArray<'js, u8>>,
    modulus_length: Option<u32>,
    public_exponent: Option<TypedArray<'js, u8>>,
    named_curve: Option<&'static str>,
}

impl<'js> HostKey<'js> {
    fn new(ctx: &Ctx<'js>, material: &KeyMaterial) -> rquickjs::Result<Self> {
        let public = material.public();
        let spki = TypedArray::new(ctx.clone(), public_der(ctx, &public)?)?;
        let private = match material {
            KeyMaterial::Private(key) => {
                let pkcs8 = key.to_pkcs8().map_err(|error| operation(ctx, &error))?;
                Some(TypedArray::new(ctx.clone(), pkcs8)?)
            }
            KeyMaterial::Public(_) => None,
        };
        let (modulus_length, public_exponent, named_curve) = match &public {
            PublicKey::Rsa(key) => {
                let exponent = TypedArray::new(ctx.clone(), keys::bytes(key.e()))?;
                (Some(key.n().bits()), Some(exponent), None)
            }
            PublicKey::Ec(key) => (None, None, Some(curves::curve_of(key).name())),
            PublicKey::Ed25519(_) | PublicKey::X25519(_) => (None, None, None),
        };
        Ok(Self {
            kind: material.kind().name(),
            public: spki,
            private,
            modulus_length,
            public_exponent,
            named_curve,
        })
    }
}

/// A key to export in `format`: its DER bytes in `keyFormat`, which must be
/// of `kind` when one is given.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct KeyExport<'js> {
    format: String,
    key_format: String,
    key: TypedArray<'js, u8>,
    kind: Option<String>,
}

#[host_function]
fn host_export_key<'js>(
    ctx: Ctx<'js>,
    options: KeyExport<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let material = der_key(
        &ctx,
        &options.key,
        &options.key_format,
        options.kind.as_deref(),
    )?;
    let output = match options.format.as_str() {
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

/// A signature scheme, `kind`, whose kind of key the key must be; the key's
/// DER bytes in `format`; the message digest; and RSA-PSS's salt length.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct Signing<'js> {
    kind: String,
    key: TypedArray<'js, u8>,
    format: String,
    hash: String,
    salt_length: Option<u32>,
}

#[host_function]
fn host_sign<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Signing<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let output = sign_key(&ctx, &input, &options)?;
    ArrayBuffer::new_copy(ctx, &output)
}

#[host_function]
fn host_verify<'js>(
    ctx: Ctx<'js>,
    signature: TypedArray<'js, u8>,
    message: TypedArray<'js, u8>,
    options: Signing<'js>,
) -> rquickjs::Result<bool> {
    let signature = bytes(&ctx, signature)?;
    let message = bytes(&ctx, message)?;
    verify_key(&ctx, &signature, &message, &options)
}

/// An RSA-OAEP operation's key, its DER bytes in `format`; its digest; and
/// its label.
#[derive(FromJs, JsType)]
pub(crate) struct Oaep<'js> {
    key: TypedArray<'js, u8>,
    format: String,
    hash: String,
    label: TypedArray<'js, u8>,
}

#[host_function]
fn host_encrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Oaep<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let plaintext = bytes(&ctx, input)?;
    let (material, hash, label) = oaep_operands(&ctx, options)?;
    let output = rsa_public(&ctx, &material.public()).and_then(|key| {
        rsa::encrypt_oaep(key, hash, &label, &plaintext).map_err(|error| operation(&ctx, &error))
    })?;
    ArrayBuffer::new_copy(ctx, &output)
}

#[host_function]
fn host_decrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: Oaep<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let ciphertext = bytes(&ctx, input)?;
    let (material, hash, label) = oaep_operands(&ctx, options)?;
    let key = rsa_private(&ctx, &material)?;
    let output = rsa::decrypt_oaep(key, hash, &label, &ciphertext)
        .map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn oaep_operands(
    ctx: &Ctx<'_>,
    options: Oaep<'_>,
) -> rquickjs::Result<(KeyMaterial, Hash, Vec<u8>)> {
    let material = der_key(ctx, &options.key, &options.format, Some("rsa-oaep"))?;
    let hash = message_digest(ctx, &options.hash)?;
    let label = bytes(ctx, options.label)?;
    Ok((material, hash, label))
}

/// A key agreement's private key, its DER bytes in `format`, which must be
/// of `kind`; and the peer's public key in SPKI.
#[derive(FromJs, JsType)]
pub(crate) struct Agreement<'js> {
    key: TypedArray<'js, u8>,
    format: String,
    kind: String,
    peer: TypedArray<'js, u8>,
}

#[host_function]
fn host_derive<'js>(ctx: Ctx<'js>, options: Agreement<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let material = der_key(&ctx, &options.key, &options.format, Some(&options.kind))?;
    let peer = bytes(&ctx, options.peer)?;
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

/// The number of checks a primality test makes.
#[derive(FromJs, JsType)]
pub(crate) struct PrimeCheck {
    checks: u32,
}

#[host_function]
fn host_check_prime<'js>(
    ctx: Ctx<'js>,
    candidate: TypedArray<'js, u8>,
    options: PrimeCheck,
) -> rquickjs::Result<bool> {
    let candidate = bytes(&ctx, candidate)?;
    if i32::try_from(options.checks).is_err() {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"checks\" is out of range",
        ));
    }
    Ok(bignum::check_prime(&candidate))
}

/// A prime to generate: its length in bits, whether it is a safe prime, and
/// the remainder `rem` it leaves when divided by `add`.
#[derive(FromJs, JsType)]
pub(crate) struct PrimeGeneration<'js> {
    bits: u32,
    safe: bool,
    add: Option<TypedArray<'js, u8>>,
    rem: Option<TypedArray<'js, u8>>,
}

#[host_function]
fn host_generate_prime<'js>(
    ctx: Ctx<'js>,
    options: PrimeGeneration<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let bits = options.bits;
    if bits < 2 || i32::try_from(bits).is_err() {
        return Err(Exception::throw_range(
            &ctx,
            "The value of \"bits\" is out of range",
        ));
    }
    let add = option_bytes(&ctx, options.add)?;
    let rem = option_bytes(&ctx, options.rem)?;
    let output = bignum::generate_prime(bits, options.safe, add.as_deref(), rem.as_deref())
        .map_err(|error| operation_error(&ctx, error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

/// scrypt's salt, its cost `n`, block size `r` and parallelism `p`, the
/// memory those may use, and the length of the derived key.
#[derive(FromJs, JsType)]
#[qjs(rename_all = "camelCase")]
pub(crate) struct ScryptParams<'js> {
    salt: TypedArray<'js, u8>,
    n: u64,
    r: u64,
    p: u64,
    maxmem: u64,
    key_length: u32,
}

#[host_function]
fn host_scrypt<'js>(
    ctx: Ctx<'js>,
    password: TypedArray<'js, u8>,
    options: ScryptParams<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let password = bytes(&ctx, password)?;
    let ScryptParams {
        salt,
        n,
        r,
        p,
        maxmem,
        key_length,
    } = options;
    let salt = bytes(&ctx, salt)?;
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
    let mut output = vec![0; key_length as usize];
    if key_length > 0 {
        scrypt::scrypt(&password, &salt, &params, &mut output)
            .map_err(|error| invalid(&error.to_string()))?;
    }
    ArrayBuffer::new_copy(ctx, &output)
}

/// The curve `WebCrypto` calls `name`.
fn parse_curve(ctx: &Ctx<'_>, name: &str) -> rquickjs::Result<Curve> {
    Curve::parse(name)
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

/// An ECDH private key: its curve, as `WebCrypto` names it, and its scalar.
#[derive(FromJs, JsType)]
pub(crate) struct EcdhKey<'js> {
    curve: String,
    private: TypedArray<'js, u8>,
}

impl EcdhKey<'_> {
    fn secret(&self, ctx: &Ctx<'_>) -> rquickjs::Result<(Curve, curves::EcSecret)> {
        let curve = parse_curve(ctx, &self.curve)?;
        let private = byte_slice(ctx, &self.private)?;
        let key =
            curves::secret_from_scalar(curve, private).map_err(|error| operation(ctx, &error))?;
        Ok((curve, key))
    }
}

#[host_function]
fn host_ecdh_public<'js>(ctx: Ctx<'js>, key: EcdhKey<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
    let (_, key) = key.secret(&ctx)?;
    ArrayBuffer::new_copy(ctx, curves::encode_point(&curves::public_of(&key), false))
}

#[host_function]
fn host_ecdh_compute<'js>(
    ctx: Ctx<'js>,
    key: EcdhKey<'js>,
    peer: TypedArray<'js, u8>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let (curve, key) = key.secret(&ctx)?;
    let peer = bytes(&ctx, peer)?;
    let peer = curves::decode_point(curve, &peer).map_err(|error| operation(&ctx, &error))?;
    let output = curves::agree(&key, &peer).map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

/// An EC point encoding: the curve, as `WebCrypto` names it, and whether the
/// point is `compressed` or `uncompressed`.
#[derive(FromJs, JsType)]
pub(crate) struct PointEncoding {
    curve: String,
    format: String,
}

#[host_function]
fn host_ecdh_convert<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: PointEncoding,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let curve = parse_curve(&ctx, &options.curve)?;
    let point = curves::decode_point(curve, &input).map_err(|error| operation(&ctx, &error))?;
    let compressed = point_format(&ctx, &options.format)?;
    ArrayBuffer::new_copy(ctx, curves::encode_point(&point, compressed))
}

/// The Diffie-Hellman group to generate: its prime's length in bits, and its
/// generator.
#[derive(FromJs, JsType)]
pub(crate) struct DhGroup {
    bits: u32,
    generator: u32,
}

/// A Diffie-Hellman group's prime and generator, big-endian.
#[derive(IntoJs, JsType)]
pub(crate) struct DhParams<'js> {
    prime: TypedArray<'js, u8>,
    generator: TypedArray<'js, u8>,
}

#[host_function]
fn host_dh_params(ctx: Ctx<'_>, options: DhGroup) -> rquickjs::Result<DhParams<'_>> {
    let (prime, generator) = bignum::dh_parameters(options.bits, options.generator)
        .map_err(|error| operation_error(&ctx, error))?;
    Ok(DhParams {
        prime: TypedArray::new(ctx.clone(), prime)?,
        generator: TypedArray::new(ctx, generator)?,
    })
}

/// A Diffie-Hellman key pair to make in the group of `prime` and
/// `generator`, from `private` when one is given.
#[derive(FromJs, JsType)]
pub(crate) struct DhGeneration<'js> {
    prime: TypedArray<'js, u8>,
    generator: TypedArray<'js, u8>,
    private: Option<TypedArray<'js, u8>>,
}

/// A Diffie-Hellman key pair, big-endian.
#[derive(IntoJs, JsType)]
pub(crate) struct DhKeys<'js> {
    public: TypedArray<'js, u8>,
    private: TypedArray<'js, u8>,
}

#[host_function]
fn host_dh_generate<'js>(
    ctx: Ctx<'js>,
    options: DhGeneration<'js>,
) -> rquickjs::Result<DhKeys<'js>> {
    let prime = bytes(&ctx, options.prime)?;
    let generator = bytes(&ctx, options.generator)?;
    let failed = |error| operation_error(&ctx, error);
    let private = match option_bytes(&ctx, options.private)? {
        Some(private) => private,
        None => bignum::random_below(&prime).map_err(failed)?,
    };
    let public = bignum::mod_pow(&generator, &private, &prime).map_err(failed)?;
    Ok(DhKeys {
        public: TypedArray::new(ctx.clone(), public)?,
        private: TypedArray::new(ctx, private)?,
    })
}

/// A Diffie-Hellman agreement in the group of `prime`, between `private` and
/// the peer's public key.
#[derive(FromJs, JsType)]
pub(crate) struct DhAgreement<'js> {
    prime: TypedArray<'js, u8>,
    private: TypedArray<'js, u8>,
    peer: TypedArray<'js, u8>,
}

#[host_function]
fn host_dh_compute<'js>(
    ctx: Ctx<'js>,
    options: DhAgreement<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let prime = bytes(&ctx, options.prime)?;
    let private = bytes(&ctx, options.private)?;
    let peer = bytes(&ctx, options.peer)?;
    let secret =
        bignum::mod_pow(&peer, &private, &prime).map_err(|error| operation_error(&ctx, error))?;
    ArrayBuffer::new_copy(ctx, &secret)
}

/// An RSA key for a raw operation: its DER bytes in `format`, which must be
/// of `kind`, and the padding, PKCS#1 when absent.
#[derive(FromJs, JsType)]
pub(crate) struct RsaLegacy<'js> {
    kind: String,
    format: String,
    key: TypedArray<'js, u8>,
    padding: Option<u32>,
}

impl RsaLegacy<'_> {
    fn key(&self, ctx: &Ctx<'_>) -> rquickjs::Result<KeyMaterial> {
        der_key(ctx, &self.key, &self.format, Some(&self.kind))
    }

    fn padding(&self, ctx: &Ctx<'_>) -> rquickjs::Result<rsa::Padding> {
        rsa::Padding::parse(self.padding).ok_or_else(|| {
            not_supported_error(ctx, "The requested RSA padding mode is not supported")
        })
    }
}

#[host_function]
fn host_rsa_legacy_private_encrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: RsaLegacy<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let plaintext = bytes(&ctx, input)?;
    let material = options.key(&ctx)?;
    let key = rsa_private(&ctx, &material)?;
    let padding = options.padding(&ctx)?;
    let output =
        rsa::private_encrypt(key, padding, &plaintext).map_err(|error| operation(&ctx, &error))?;
    ArrayBuffer::new_copy(ctx, &output)
}

#[host_function]
fn host_rsa_legacy_public_decrypt<'js>(
    ctx: Ctx<'js>,
    input: TypedArray<'js, u8>,
    options: RsaLegacy<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let ciphertext = bytes(&ctx, input)?;
    let material = options.key(&ctx)?;
    let padding = options.padding(&ctx)?;
    let output = rsa_public(&ctx, &material.public()).and_then(|key| {
        rsa::public_decrypt(key, padding, &ciphertext).map_err(|error| operation(&ctx, &error))
    })?;
    ArrayBuffer::new_copy(ctx, &output)
}

fn generate_key(ctx: &Ctx<'_>, options: KeyGeneration<'_>) -> rquickjs::Result<PrivateKey> {
    Ok(match options.kind.as_str() {
        "rsa" | "rsa-pss" | "rsa-oaep" => {
            let modulus_length = required(ctx, options.modulus_length, "modulusLength")?;
            let exponent = required(ctx, options.public_exponent, "publicExponent")?;
            PrivateKey::Rsa(
                rsa::generate(modulus_length as usize, &bytes(ctx, exponent)?)
                    .map_err(|error| operation(ctx, &error))?,
            )
        }
        "ec" => {
            let curve = required(ctx, options.curve, "curve")?;
            PrivateKey::Ec(curves::generate(parse_curve(ctx, &curve)?))
        }
        "ed25519" => PrivateKey::Ed25519(ed25519_dalek::SigningKey::generate(&mut rng())),
        "x25519" => PrivateKey::X25519(x25519_dalek::StaticSecret::random_from_rng(&mut rng())),
        _ => return Err(unsupported_algorithm(ctx)),
    })
}

/// The key DER-encoded in `key` as `format`, which must be of `kind` when one
/// is given.
fn der_key(
    ctx: &Ctx<'_>,
    key: &TypedArray<'_, u8>,
    format: &str,
    kind: Option<&str>,
) -> rquickjs::Result<KeyMaterial> {
    of_kind(ctx, parse_der(ctx, byte_slice(ctx, key)?, format)?, kind)
}

/// The key DER-encoded in `input` as `format`: SPKI, PKCS#8, or `der` for
/// whichever of them `input` is.
fn parse_der(ctx: &Ctx<'_>, input: &[u8], format: &str) -> rquickjs::Result<KeyMaterial> {
    let material = match format {
        "spki" => PublicKey::from_spki(input).map(KeyMaterial::Public),
        "pkcs8" => PrivateKey::from_pkcs8(input).map(KeyMaterial::Private),
        "der" => PrivateKey::from_pkcs8(input)
            .map(KeyMaterial::Private)
            .or_else(|_| PublicKey::from_spki(input).map(KeyMaterial::Public)),
        _ => return Err(unsupported_format(ctx)),
    };
    material.map_err(|error| key_error(ctx, error))
}

/// The raw public key `input`, of the kind `kind` names; an EC key's `curve`
/// names its curve.
fn raw_key(
    ctx: &Ctx<'_>,
    input: &[u8],
    kind: Option<&str>,
    curve: Option<&str>,
) -> rquickjs::Result<KeyMaterial> {
    let Some(kind) = kind.and_then(Kind::parse).filter(|kind| *kind != Kind::Rsa) else {
        return Err(not_supported_error(
            ctx,
            "Raw import is not supported for this algorithm",
        ));
    };
    let curve = curve.and_then(Curve::parse);
    PublicKey::from_raw(kind, curve, input)
        .map(KeyMaterial::Public)
        .map_err(|error| key_error(ctx, error))
}

/// The key in the JSON Web Key `text`, of the kind `kind` names.
fn jwk_key(
    ctx: &Ctx<'_>,
    kind: Option<&str>,
    text: Option<String>,
) -> rquickjs::Result<KeyMaterial> {
    let text = required(ctx, text, "jwk")?;
    let jwk: JsonValue =
        serde_json::from_str(&text).map_err(|error| data_error(ctx, &error.to_string()))?;
    let kind = kind
        .and_then(Kind::parse)
        .ok_or_else(|| data_error(ctx, "The JWK does not match the requested algorithm"))?;
    keys::from_jwk(kind, &jwk).map_err(|error| key_error(ctx, error))
}

/// `material`, which must be of the kind `kind` names when one is given.
fn of_kind(
    ctx: &Ctx<'_>,
    material: KeyMaterial,
    kind: Option<&str>,
) -> rquickjs::Result<KeyMaterial> {
    if kind.is_some_and(|name| Kind::parse(name) != Some(material.kind())) {
        return Err(data_error(
            ctx,
            "The key does not match the requested algorithm",
        ));
    }
    Ok(material)
}

fn key_error(ctx: &Ctx<'_>, error: KeyError) -> rquickjs::Error {
    match error {
        KeyError::Data(message) => data_error(ctx, &message),
        KeyError::Operation(message) => operation_error(ctx, &message),
    }
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

fn sign_key(ctx: &Ctx<'_>, message: &[u8], options: &Signing<'_>) -> rquickjs::Result<Vec<u8>> {
    let material = der_key(ctx, &options.key, &options.format, Some(&options.kind))?;
    let hash = || message_digest(ctx, &options.hash);
    let KeyMaterial::Private(key) = &material else {
        return Err(private_required(ctx));
    };
    let failed = |error: KeyError| operation(ctx, &error);
    match (options.kind.as_str(), key) {
        ("rsa", PrivateKey::Rsa(key)) => rsa::sign_pkcs1(key, hash()?, message).map_err(failed),
        ("rsa-pss", PrivateKey::Rsa(key)) => {
            let salt_length = required(ctx, options.salt_length, "saltLength")? as usize;
            rsa::sign_pss(key, hash()?, salt_length, message).map_err(failed)
        }
        ("ec" | "ecdsa", PrivateKey::Ec(key)) => {
            curves::sign(key, &hash()?.digest(message)).map_err(failed)
        }
        ("ed25519", PrivateKey::Ed25519(key)) => Ok(curves::ed25519_sign(key, message)),
        _ => Err(unsupported_algorithm(ctx)),
    }
}

fn verify_key(
    ctx: &Ctx<'_>,
    signature: &[u8],
    message: &[u8],
    options: &Signing<'_>,
) -> rquickjs::Result<bool> {
    let key = der_key(ctx, &options.key, &options.format, Some(&options.kind))?.public();
    let hash = || message_digest(ctx, &options.hash);
    match (options.kind.as_str(), &key) {
        ("rsa", PublicKey::Rsa(key)) => Ok(rsa::verify_pkcs1(key, hash()?, message, signature)),
        ("rsa-pss", PublicKey::Rsa(key)) => {
            let salt_length = required(ctx, options.salt_length, "saltLength")? as usize;
            Ok(rsa::verify_pss(
                key,
                hash()?,
                salt_length,
                message,
                signature,
            ))
        }
        ("ec" | "ecdsa", PublicKey::Ec(key)) => {
            Ok(curves::verify(key, &hash()?.digest(message), signature))
        }
        ("ed25519", PublicKey::Ed25519(key)) => Ok(curves::ed25519_verify(key, message, signature)),
        _ => Err(unsupported_algorithm(ctx)),
    }
}

fn message_digest(ctx: &Ctx<'_>, algorithm: &str) -> rquickjs::Result<Hash> {
    Hash::parse(algorithm).ok_or_else(|| unsupported_algorithm(ctx))
}

/// `value`, which the option `name` must give.
fn required<T>(ctx: &Ctx<'_>, value: Option<T>, name: &str) -> rquickjs::Result<T> {
    value.ok_or_else(|| Exception::throw_type(ctx, &format!("{name} is required")))
}

fn optional_bytes(ctx: &Ctx<'_>, value: Option<TypedArray<'_, u8>>) -> rquickjs::Result<Vec<u8>> {
    Ok(option_bytes(ctx, value)?.unwrap_or_default())
}

fn option_bytes(
    ctx: &Ctx<'_>,
    value: Option<TypedArray<'_, u8>>,
) -> rquickjs::Result<Option<Vec<u8>>> {
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

/// The bytes the hexadecimal `text` spells.
#[cfg(test)]
fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16))
        .collect::<Result<_, _>>()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use rquickjs::function::Opt;

    #[test]
    fn hmac_accepts_an_empty_key() -> rquickjs::Result<()> {
        let runtime = rquickjs::Runtime::new()?;
        let context = rquickjs::Context::full(&runtime)?;
        context.with(|ctx| {
            let key = TypedArray::new(ctx.clone(), Vec::<u8>::new())?;
            let input = TypedArray::new(ctx.clone(), b"abc".to_vec())?;
            let state = digest::create(ctx.clone(), "SHA-256".into(), Opt(Some(key)))?;
            state.borrow_mut().update(input)?;
            let output = state.borrow_mut().finish(ctx.clone())?;
            let bytes = output
                .as_bytes()
                .ok_or_else(|| crate::errors::throw_message(&ctx, "detached ArrayBuffer"))?;
            // Pinned Workerd createHmac('sha256', '').update('abc').digest('base64').
            assert_eq!(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                "/XrbFSwF74Dcz1Ch+kwF1aPsbalVdfwxKufF0JGDY1E="
            );
            Ok(())
        })
    }
}
