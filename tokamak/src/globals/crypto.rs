#![allow(clippy::needless_pass_by_value)]

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openssl::bn::{BigNum, BigNumContext};
use openssl::derive::Deriver;
use openssl::dh::Dh;
use openssl::ec::{EcGroup, EcKey, EcPoint, PointConversionForm};
use openssl::ecdsa::EcdsaSig;
use openssl::encrypt::{Decrypter, Encrypter};
use openssl::hash::{MessageDigest, hash};
use openssl::nid::Nid;
use openssl::pkcs5;
use openssl::pkey::{HasPrivate, HasPublic, Id, PKey, Private, Public};
use openssl::rsa::{Padding, Rsa};
use openssl::sign::{RsaPssSaltlen, Signer, Verifier};
use openssl::symm::{Cipher, Crypter, Mode};
use rquickjs::{ArrayBuffer, Constructor, Ctx, Exception, Object, TypedArray, Value};
use serde_json::{Value as JsonValue, json};

pub(super) fn digest<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    input: TypedArray<'js, u8>,
    options: Option<Object<'js>>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let input = bytes(&ctx, input)?;
    let output = match options {
        Some(options) => crypto_operation(&ctx, &input, &options)?,
        None => {
            let message_digest = message_digest(&ctx, &algorithm)?;
            hash(message_digest, &input)
                .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
                .to_vec()
        }
    };
    ArrayBuffer::new_copy(ctx, &output)
}

pub(super) fn hmac<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    key: TypedArray<'js, u8>,
    input: TypedArray<'js, u8>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let message_digest = message_digest(&ctx, &algorithm)?;
    let key = bytes(&ctx, key)?;
    let input = bytes(&ctx, input)?;
    let key =
        PKey::hmac(&key).map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    let mut signer = Signer::new(message_digest, &key)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    signer
        .update(&input)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    let output = signer
        .sign_to_vec()
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    ArrayBuffer::new_copy(ctx, &output)
}

pub(super) fn aes_gcm<'js>(
    ctx: Ctx<'js>,
    encrypt: bool,
    key: TypedArray<'js, u8>,
    iv: TypedArray<'js, u8>,
    additional_data: TypedArray<'js, u8>,
    input: TypedArray<'js, u8>,
    tag_options: Value<'js>,
) -> rquickjs::Result<ArrayBuffer<'js>> {
    let key = bytes(&ctx, key)?;
    let iv = bytes(&ctx, iv)?;
    let additional_data = bytes(&ctx, additional_data)?;
    let input = bytes(&ctx, input)?;
    let (tag_length, mode) = aes_options(&ctx, tag_options)?;
    let output = match mode.as_str() {
        "AES-GCM" => aes_gcm_operation(
            &ctx,
            encrypt,
            &key,
            &iv,
            &additional_data,
            &input,
            tag_length,
        )?,
        "AES-CBC" => aes_cbc_operation(&ctx, encrypt, &key, &iv, &input)?,
        "AES-CTR" => aes_ctr_operation(&ctx, &key, &iv, &input, tag_length)?,
        "AES-KW" => aes_kw_operation(&ctx, encrypt, &key, &input)?,
        _ => {
            return Err(throw_dom_exception(
                &ctx,
                "NotSupportedError",
                "The requested cryptographic algorithm is not supported",
            ));
        }
    };
    ArrayBuffer::new_copy(ctx, &output)
}

fn aes_gcm_operation(
    ctx: &Ctx<'_>,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    additional_data: &[u8],
    input: &[u8],
    tag_length: u32,
) -> rquickjs::Result<Vec<u8>> {
    let tag_bytes = tag_length
        .checked_div(8)
        .filter(|length| matches!(*length, 4 | 8 | 12 | 13 | 14 | 15 | 16))
        .ok_or_else(|| throw_dom_exception(ctx, "OperationError", "Invalid AES-GCM tag length"))?
        as usize;
    let cipher = aes_cipher(key, CipherKind::Gcm, ctx)?;
    let (input, tag) = if encrypt {
        (input, None)
    } else if input.len() < tag_bytes {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "The ciphertext is too short",
        ));
    } else {
        let split = input.len() - tag_bytes;
        (&input[..split], Some(&input[split..]))
    };
    let mut crypter = Crypter::new(
        cipher,
        if encrypt {
            Mode::Encrypt
        } else {
            Mode::Decrypt
        },
        key,
        Some(iv),
    )
    .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    crypter.pad(false);
    crypter
        .aad_update(additional_data)
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    if let Some(tag) = tag {
        crypter
            .set_tag(tag)
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    }
    let mut output = vec![0; input.len() + cipher.block_size()];
    let count = crypter
        .update(input, &mut output)
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    let count = count
        + crypter.finalize(&mut output[count..]).map_err(|error| {
            if encrypt {
                Exception::throw_internal(ctx, &error.to_string())
            } else {
                throw_dom_exception(ctx, "OperationError", &error.to_string())
            }
        })?;
    output.truncate(count);
    if encrypt {
        let mut tag = vec![0; tag_bytes];
        crypter
            .get_tag(&mut tag)
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
        output.extend_from_slice(&tag);
    }
    Ok(output)
}

fn aes_options<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<(u32, String)> {
    if value.is_object() {
        let object = value
            .into_object()
            .ok_or_else(|| Exception::throw_type(ctx, "Invalid AES options"))?;
        let tag_length: Option<u32> = object.get("tagLength")?;
        let mode: Option<String> = object.get("mode")?;
        Ok((
            tag_length.unwrap_or(128),
            mode.unwrap_or_else(|| "AES-GCM".to_owned()),
        ))
    } else {
        Ok((value.get::<u32>()?, "AES-GCM".to_owned()))
    }
}

fn aes_cbc_operation(
    ctx: &Ctx<'_>,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    if iv.len() != 16 {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "AES-CBC IV must be 16 bytes long",
        ));
    }
    let cipher = aes_cipher(key, CipherKind::Cbc, ctx)?;
    let mut crypter = Crypter::new(
        cipher,
        if encrypt {
            Mode::Encrypt
        } else {
            Mode::Decrypt
        },
        key,
        Some(iv),
    )
    .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    let mut output = vec![0; input.len() + cipher.block_size()];
    let count = crypter
        .update(input, &mut output)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let count = count
        + crypter
            .finalize(&mut output[count..])
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.truncate(count);
    Ok(output)
}

fn aes_ctr_operation(
    ctx: &Ctx<'_>,
    key: &[u8],
    counter: &[u8],
    input: &[u8],
    length: u32,
) -> rquickjs::Result<Vec<u8>> {
    if counter.len() != 16 || !(1..=128).contains(&length) {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "Invalid AES-CTR counter or length",
        ));
    }
    let mut counter_value = [0u8; 16];
    counter_value.copy_from_slice(counter);
    let mut output = vec![0; input.len()];
    for (index, chunk) in input.chunks(16).enumerate() {
        if index > 0 && !increment_counter(&mut counter_value, length as usize) {
            return Err(throw_dom_exception(
                ctx,
                "OperationError",
                "AES-CTR counter overflow",
            ));
        }
        let stream = aes_block(ctx, key, &counter_value, true)?;
        let offset = index * 16;
        for (position, byte) in chunk.iter().enumerate() {
            output[offset + position] = *byte ^ stream[position];
        }
    }
    Ok(output)
}

fn increment_counter(counter: &mut [u8], length: usize) -> bool {
    for bit in 0..length {
        let position = 127 - bit;
        let byte = position / 8;
        let mask = 1 << (position % 8);
        if counter[byte] & mask != 0 {
            counter[byte] &= !mask;
        } else {
            counter[byte] |= mask;
            return true;
        }
    }
    false
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
    let n = if encrypt {
        input.len() / 8
    } else {
        input.len() / 8 - 1
    };
    let mut a = if encrypt {
        [0xa6; 8]
    } else {
        let mut value = [0; 8];
        value.copy_from_slice(&input[..8]);
        value
    };
    let mut r = if encrypt {
        input
            .chunks_exact(8)
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>()
    } else {
        input[8..]
            .chunks_exact(8)
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<_>>()
    };
    let mut process = |round: usize, index: usize| -> rquickjs::Result<()> {
        let t = (n * round + index + 1) as u64;
        let mut block = [0; 16];
        for position in 0..8 {
            block[position] = a[position] ^ (t >> (56 - position * 8)) as u8;
            block[8 + position] = r[index][position];
        }
        let decrypted = aes_block(ctx, key, &block, encrypt)?;
        a.copy_from_slice(&decrypted[..8]);
        r[index].copy_from_slice(&decrypted[8..]);
        Ok(())
    };
    if encrypt {
        for round in 0..6 {
            for index in 0..n {
                process(round, index)?;
            }
        }
    } else {
        for round in (0..6).rev() {
            for index in (0..n).rev() {
                process(round, index)?;
            }
        }
    }
    if !encrypt && a != [0xa6; 8] {
        return Err(throw_dom_exception(
            ctx,
            "OperationError",
            "AES-KW integrity check failed",
        ));
    }
    let mut output = Vec::with_capacity(if encrypt {
        input.len() + 8
    } else {
        input.len() - 8
    });
    if encrypt {
        output.extend_from_slice(&a);
    }
    for chunk in r {
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

#[derive(Clone, Copy)]
enum CipherKind {
    Gcm,
    Cbc,
    Ecb,
}

fn aes_cipher(key: &[u8], kind: CipherKind, ctx: &Ctx<'_>) -> rquickjs::Result<Cipher> {
    let cipher = match (key.len(), kind) {
        (16, CipherKind::Gcm) => Cipher::aes_128_gcm(),
        (24, CipherKind::Gcm) => Cipher::aes_192_gcm(),
        (32, CipherKind::Gcm) => Cipher::aes_256_gcm(),
        (16, CipherKind::Cbc) => Cipher::aes_128_cbc(),
        (24, CipherKind::Cbc) => Cipher::aes_192_cbc(),
        (32, CipherKind::Cbc) => Cipher::aes_256_cbc(),
        (16, CipherKind::Ecb) => Cipher::aes_128_ecb(),
        (24, CipherKind::Ecb) => Cipher::aes_192_ecb(),
        (32, CipherKind::Ecb) => Cipher::aes_256_ecb(),
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "DataError",
                "AES keys must be 128, 192, or 256 bits",
            ));
        }
    };
    Ok(cipher)
}

fn aes_block(
    ctx: &Ctx<'_>,
    key: &[u8],
    block: &[u8; 16],
    encrypt: bool,
) -> rquickjs::Result<[u8; 16]> {
    let cipher = aes_cipher(key, CipherKind::Ecb, ctx)?;
    let mut crypter = Crypter::new(
        cipher,
        if encrypt {
            Mode::Encrypt
        } else {
            Mode::Decrypt
        },
        key,
        None,
    )
    .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    crypter.pad(false);
    let mut output = [0; 32];
    let count = crypter
        .update(block, &mut output)
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    let count = count
        + crypter
            .finalize(&mut output[count..])
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    if count != 16 {
        return Err(Exception::throw_internal(
            ctx,
            "AES block operation returned an invalid length",
        ));
    }
    output[..16].try_into().map_err(|_| {
        Exception::throw_internal(ctx, "AES block operation returned an invalid length")
    })
}

fn crypto_operation(
    ctx: &Ctx<'_>,
    input: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<Vec<u8>> {
    let operation: String = options.get("operation")?;
    match operation.as_str() {
        "generate" => generate_key(ctx, options),
        "import" => import_key(ctx, input, options),
        "export" => export_key(ctx, input, options),
        "sign" => sign_key(ctx, input, options),
        "verify" => Ok(vec![u8::from(verify_key(ctx, input, options)?)]),
        "encrypt" => encrypt_key(ctx, input, options),
        "decrypt" => decrypt_key(ctx, input, options),
        "derive" => derive_key(ctx, input, options),
        "checkPrime" => check_prime(ctx, input, options),
        "generatePrime" => generate_prime(ctx, options),
        "scrypt" => scrypt(ctx, input, options),
        "ecdhPublic" => ecdh_public(ctx, options),
        "ecdhCompute" => ecdh_compute(ctx, options),
        "ecdhConvert" => ecdh_convert(ctx, input, options),
        "dhParams" => dh_params_generate(ctx, options),
        "dhGenerate" => dh_generate(ctx, options),
        "dhCompute" => dh_compute(ctx, options),
        "rsaLegacyPrivateEncrypt" => rsa_legacy_private_encrypt(ctx, input, options),
        "rsaLegacyPublicDecrypt" => rsa_legacy_public_decrypt(ctx, input, options),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic operation is not supported",
        )),
    }
}

fn check_prime(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let checks_value: Option<u32> = options.get("checks")?;
    let checks: i32 = checks_value.unwrap_or(0).try_into().map_err(|_| {
        Exception::throw_range(ctx, "The value of \"checks\" is out of range")
    })?;
    let candidate = BigNum::from_slice(input)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let prime = candidate
        .is_prime(checks, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok(vec![u8::from(prime)])
}

fn generate_prime(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let bits = required_u32(ctx, options, "bits")?;
    let bits: i32 = bits.try_into().map_err(|_| {
        Exception::throw_range(ctx, "The value of \"bits\" is out of range")
    })?;
    if bits < 2 {
        return Err(Exception::throw_range(ctx, "The value of \"bits\" is out of range"));
    }
    let safe_value: Option<bool> = options.get("safe")?;
    let safe = safe_value.unwrap_or(false);
    let add = option_bytes(ctx, options, "add")?
        .map(|value| BigNum::from_slice(&value))
        .transpose()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let rem = option_bytes(ctx, options, "rem")?
        .map(|value| BigNum::from_slice(&value))
        .transpose()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut prime = BigNum::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    prime
        .generate_prime(bits, safe, add.as_deref(), rem.as_deref())
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok(prime.to_vec())
}

fn scrypt(ctx: &Ctx<'_>, password: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let salt = required_bytes(ctx, options, "salt")?;
    let n_value: Option<u64> = options.get("n")?;
    let r_value: Option<u64> = options.get("r")?;
    let p_value: Option<u64> = options.get("p")?;
    let maxmem_value: Option<u64> = options.get("maxmem")?;
    let n = n_value.unwrap_or(16_384);
    let r = r_value.unwrap_or(8);
    let p = p_value.unwrap_or(1);
    let maxmem = maxmem_value.unwrap_or(32 * 1024 * 1024);
    let length = required_u32(ctx, options, "keyLength")? as usize;
    let mut output = vec![0; length];
    pkcs5::scrypt(password, &salt, n, r, p, maxmem, &mut output)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok(output)
}

fn node_curve(ctx: &Ctx<'_>, curve: &str) -> rquickjs::Result<String> {
    match curve {
        "prime256v1" => Ok("P-256".to_owned()),
        "secp384r1" => Ok("P-384".to_owned()),
        "secp521r1" => Ok("P-521".to_owned()),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested elliptic curve is not supported",
        )),
    }
}

fn ecdh_private_key(
    ctx: &Ctx<'_>,
    curve: &str,
    private: &[u8],
) -> rquickjs::Result<(EcGroup, EcKey<Private>)> {
    let curve = node_curve(ctx, curve)?;
    let group = ec_group(ctx, &curve)?;
    let private = BigNum::from_slice(private)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut public = EcPoint::new(&group)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    public
        .mul_generator2(&group, &private, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let key = EcKey::from_private_components(&group, &private, &public)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok((group, key))
}

fn point_bytes(
    ctx: &Ctx<'_>,
    group: &openssl::ec::EcGroupRef,
    point: &openssl::ec::EcPointRef,
    format: &str,
) -> rquickjs::Result<Vec<u8>> {
    let form = match format {
        "compressed" => PointConversionForm::COMPRESSED,
        "uncompressed" => PointConversionForm::UNCOMPRESSED,
        _ => {
            return Err(Exception::throw_type(
                ctx,
                "The point format must be \"compressed\" or \"uncompressed\"",
            ));
        }
    };
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    point
        .to_bytes(group, form, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn ecdh_public(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let curve = required_string(ctx, options, "curve")?;
    let private = required_bytes(ctx, options, "private")?;
    let (group, key) = ecdh_private_key(ctx, &curve, &private)?;
    let format = option_string(options, "format")?.unwrap_or_else(|| "uncompressed".to_owned());
    point_bytes(ctx, &group, key.public_key(), &format)
}

fn ecdh_compute(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let curve = required_string(ctx, options, "curve")?;
    let private = required_bytes(ctx, options, "private")?;
    let peer = required_bytes(ctx, options, "peer")?;
    let (group, key) = ecdh_private_key(ctx, &curve, &private)?;
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let peer = EcPoint::from_bytes(&group, &peer, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let peer = EcKey::from_public_key(&group, &peer)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let private = PKey::from_ec_key(key)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let peer = PKey::from_ec_key(peer)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut deriver = Deriver::new(&private)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    deriver
        .set_peer(&peer)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    deriver
        .derive_to_vec()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn ecdh_convert(
    ctx: &Ctx<'_>,
    input: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<Vec<u8>> {
    let curve = required_string(ctx, options, "curve")?;
    let curve = node_curve(ctx, &curve)?;
    let group = ec_group(ctx, &curve)?;
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let point = EcPoint::from_bytes(&group, input, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let format = required_string(ctx, options, "format")?;
    point_bytes(ctx, &group, &point, &format)
}

fn dh_params_generate(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let bits = required_u32(ctx, options, "bits")?;
    let generator_value: Option<u32> = options.get("generator")?;
    let generator = generator_value.unwrap_or(2);
    let params = Dh::generate_params(bits, generator)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let generator = params.generator().to_vec();
    bundle(&params.prime_p().to_vec(), Some(&generator))
}

fn dh_generate(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let prime = BigNum::from_slice(&required_bytes(ctx, options, "prime")?)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let generator = BigNum::from_slice(&required_bytes(ctx, options, "generator")?)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let private = match option_bytes(ctx, options, "private")? {
        Some(private) => BigNum::from_slice(&private)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        None => {
            let mut private = BigNum::new()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            loop {
                prime
                    .rand_range(&mut private)
                    .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
                let value = private.to_vec();
                if value.len() > 1 || value.first().is_some_and(|byte| *byte > 1) {
                    break private;
                }
            }
        }
    };
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut public = BigNum::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    public
        .mod_exp(&generator, &private, &prime, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    bundle(&public.to_vec(), Some(&private.to_vec()))
}

fn dh_compute(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let prime = BigNum::from_slice(&required_bytes(ctx, options, "prime")?)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let private = BigNum::from_slice(&required_bytes(ctx, options, "private")?)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let peer = BigNum::from_slice(&required_bytes(ctx, options, "peer")?)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut secret = BigNum::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    secret
        .mod_exp(&peer, &private, &prime, &mut context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok(secret.to_vec())
}

fn rsa_legacy_private_encrypt(
    ctx: &Ctx<'_>,
    input: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<Vec<u8>> {
    let key_bytes = message_key_bytes(ctx, options)?;
    let KeyMaterial::Private(key) = parse_key(ctx, &key_bytes, options)? else {
        return Err(throw_dom_exception(ctx, "InvalidAccessError", "A private key is required"));
    };
    let rsa = key
        .rsa()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let padding = rsa_padding(ctx, options)?;
    let mut output = vec![0; rsa.size() as usize];
    let length = rsa
        .private_encrypt(input, &mut output, padding)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.truncate(length);
    Ok(output)
}

fn rsa_legacy_public_decrypt(
    ctx: &Ctx<'_>,
    input: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<Vec<u8>> {
    let key_bytes = message_key_bytes(ctx, options)?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let padding = rsa_padding(ctx, options)?;
    match material {
        KeyMaterial::Private(key) => {
            let rsa = key
                .rsa()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            rsa_legacy_public_decrypt_with(ctx, input, &rsa, padding)
        }
        KeyMaterial::Public(key) => {
            let rsa = key
                .rsa()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            rsa_legacy_public_decrypt_with(ctx, input, &rsa, padding)
        }
    }
}

fn rsa_legacy_public_decrypt_with<T: HasPublic>(
    ctx: &Ctx<'_>,
    input: &[u8],
    rsa: &Rsa<T>,
    padding: Padding,
) -> rquickjs::Result<Vec<u8>> {
    let mut output = vec![0; rsa.size() as usize];
    let length = rsa
        .public_decrypt(input, &mut output, padding)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.truncate(length);
    Ok(output)
}

fn rsa_padding(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Padding> {
    let padding: Option<u32> = options.get("padding")?;
    match padding {
        None | Some(1) => Ok(Padding::PKCS1),
        Some(3) => Ok(Padding::NONE),
        Some(4) => Ok(Padding::PKCS1_OAEP),
        Some(_) => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested RSA padding mode is not supported",
        )),
    }
}

enum KeyMaterial {
    Private(PKey<Private>),
    Public(PKey<Public>),
}

fn generate_key(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let kind = required_string(ctx, options, "kind")?;
    let material = match kind.as_str() {
        "rsa" | "rsa-pss" | "rsa-oaep" => {
            let modulus_length = required_u32(ctx, options, "modulusLength")?;
            let exponent = required_bytes(ctx, options, "publicExponent")?;
            let exponent = BigNum::from_slice(&exponent)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
            let rsa = Rsa::generate_with_e(modulus_length, &exponent)
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            KeyMaterial::Private(
                PKey::from_rsa(rsa).map_err(|error| {
                    throw_dom_exception(ctx, "OperationError", &error.to_string())
                })?,
            )
        }
        "ec" => {
            let curve = required_string(ctx, options, "curve")?;
            let group = ec_group(ctx, &curve)?;
            let key = EcKey::generate(&group)
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            KeyMaterial::Private(
                PKey::from_ec_key(key).map_err(|error| {
                    throw_dom_exception(ctx, "OperationError", &error.to_string())
                })?,
            )
        }
        "ed25519" => KeyMaterial::Private(
            PKey::generate_ed25519()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        ),
        "x25519" => KeyMaterial::Private(
            PKey::generate_x25519()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        ),
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "NotSupportedError",
                "The requested cryptographic algorithm is not supported",
            ));
        }
    };
    bundle_private(ctx, &material)
}

fn import_key(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let input = key_input(ctx, input, options)?;
    let material = parse_key(ctx, &input, options)?;
    match material {
        KeyMaterial::Private(key) => bundle_private(ctx, &KeyMaterial::Private(key)),
        KeyMaterial::Public(key) => bundle_public(ctx, &KeyMaterial::Public(key)),
    }
}

fn export_key(ctx: &Ctx<'_>, input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let format = required_string(ctx, options, "format")?;
    let parse_format = option_string(options, "keyFormat")?.unwrap_or_else(|| format.clone());
    let input = key_input(ctx, input, options)?;
    let material = parse_key_with_format(ctx, &input, options, &parse_format)?;
    match format.as_str() {
        "spki" => material_public_der(ctx, &material),
        "pkcs8" => material_private_der(ctx, &material),
        "raw" => material_raw_public(ctx, &material),
        "jwk" => material_jwk(ctx, &material),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested key format is not supported",
        )),
    }
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
    let kind = required_string(ctx, options, "kind")?;
    let material = match format {
        "spki" => KeyMaterial::Public(
            PKey::public_key_from_der(input)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
        ),
        "pkcs8" => KeyMaterial::Private(
            PKey::private_key_from_pkcs8(input)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
        ),
        "raw" => match kind.as_str() {
            "ec" => KeyMaterial::Public(ec_public_from_raw(ctx, input, options)?),
            "ed25519" => KeyMaterial::Public(
                PKey::public_key_from_raw_bytes(input, Id::ED25519)
                    .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
            ),
            "x25519" => KeyMaterial::Public(
                PKey::public_key_from_raw_bytes(input, Id::X25519)
                    .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
            ),
            _ => {
                return Err(throw_dom_exception(
                    ctx,
                    "NotSupportedError",
                    "Raw import is not supported for this algorithm",
                ));
            }
        },
        "jwk" => parse_jwk(ctx, options, &kind)?,
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "NotSupportedError",
                "The requested key format is not supported",
            ));
        }
    };
    ensure_kind(ctx, &material, &kind)?;
    Ok(material)
}

fn parse_jwk(ctx: &Ctx<'_>, options: &Object<'_>, kind: &str) -> rquickjs::Result<KeyMaterial> {
    let text = required_string(ctx, options, "jwk")?;
    let jwk: JsonValue = serde_json::from_str(&text)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    let key_type = jwk
        .get("kty")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    match kind {
        "rsa" | "rsa-pss" | "rsa-oaep" if key_type == "RSA" => parse_rsa_jwk(ctx, &jwk),
        "ec" if key_type == "EC" => parse_ec_jwk(ctx, &jwk),
        "ed25519" if key_type == "OKP" => parse_okp_jwk(ctx, &jwk, "Ed25519", Id::ED25519),
        "x25519" if key_type == "OKP" => parse_okp_jwk(ctx, &jwk, "X25519", Id::X25519),
        _ => Err(throw_dom_exception(
            ctx,
            "DataError",
            "The JWK does not match the requested algorithm",
        )),
    }
}

fn parse_rsa_jwk(ctx: &Ctx<'_>, jwk: &JsonValue) -> rquickjs::Result<KeyMaterial> {
    let n = jwk_bytes(ctx, jwk, "n")?;
    let e = jwk_bytes(ctx, jwk, "e")?;
    let n = BigNum::from_slice(&n)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    let e = BigNum::from_slice(&e)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    if jwk.get("d").is_some() {
        let d = jwk_big_num(ctx, jwk, "d")?;
        let p = jwk_big_num(ctx, jwk, "p")?;
        let q = jwk_big_num(ctx, jwk, "q")?;
        let dp = jwk_big_num(ctx, jwk, "dp")?;
        let dq = jwk_big_num(ctx, jwk, "dq")?;
        let qi = jwk_big_num(ctx, jwk, "qi")?;
        let rsa = Rsa::from_private_components(n, e, d, p, q, dp, dq, qi)
            .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
        Ok(KeyMaterial::Private(PKey::from_rsa(rsa).map_err(
            |error| throw_dom_exception(ctx, "DataError", &error.to_string()),
        )?))
    } else {
        let rsa = Rsa::from_public_components(n, e)
            .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
        Ok(KeyMaterial::Public(PKey::from_rsa(rsa).map_err(
            |error| throw_dom_exception(ctx, "DataError", &error.to_string()),
        )?))
    }
}

fn parse_ec_jwk(ctx: &Ctx<'_>, jwk: &JsonValue) -> rquickjs::Result<KeyMaterial> {
    let curve = jwk_string(ctx, jwk, "crv")?;
    let group = ec_group(ctx, &curve)?;
    let x = jwk_big_num(ctx, jwk, "x")?;
    let y = jwk_big_num(ctx, jwk, "y")?;
    if jwk.get("d").is_some() {
        let d = jwk_big_num(ctx, jwk, "d")?;
        let key = EcKey::from_public_key_affine_coordinates(&group, &x, &y)
            .and_then(|public| EcKey::from_private_components(&group, &d, public.public_key()))
            .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
        Ok(KeyMaterial::Private(PKey::from_ec_key(key).map_err(
            |error| throw_dom_exception(ctx, "DataError", &error.to_string()),
        )?))
    } else {
        let key = EcKey::from_public_key_affine_coordinates(&group, &x, &y)
            .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
        Ok(KeyMaterial::Public(PKey::from_ec_key(key).map_err(
            |error| throw_dom_exception(ctx, "DataError", &error.to_string()),
        )?))
    }
}

fn parse_okp_jwk(
    ctx: &Ctx<'_>,
    jwk: &JsonValue,
    curve: &str,
    id: Id,
) -> rquickjs::Result<KeyMaterial> {
    if jwk_string(ctx, jwk, "crv")? != curve {
        return Err(throw_dom_exception(
            ctx,
            "DataError",
            "The JWK curve does not match the requested algorithm",
        ));
    }
    if jwk.get("d").is_some() {
        let d = jwk_bytes(ctx, jwk, "d")?;
        Ok(KeyMaterial::Private(
            PKey::private_key_from_raw_bytes(&d, id)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
        ))
    } else {
        let x = jwk_bytes(ctx, jwk, "x")?;
        Ok(KeyMaterial::Public(
            PKey::public_key_from_raw_bytes(&x, id)
                .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?,
        ))
    }
}

fn ec_public_from_raw(
    ctx: &Ctx<'_>,
    raw: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<PKey<Public>> {
    let curve = required_string(ctx, options, "curve")?;
    let group = ec_group(ctx, &curve)?;
    let mut bn_context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    let point = EcPoint::from_bytes(&group, raw, &mut bn_context)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    let key = EcKey::from_public_key(&group, &point)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))?;
    PKey::from_ec_key(key)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))
}

fn ensure_kind(ctx: &Ctx<'_>, material: &KeyMaterial, kind: &str) -> rquickjs::Result<()> {
    let id = match material {
        KeyMaterial::Private(key) => key.id(),
        KeyMaterial::Public(key) => key.id(),
    };
    let expected = match kind {
        "rsa" | "rsa-pss" | "rsa-oaep" => id == Id::RSA,
        "ec" => id == Id::EC,
        "ed25519" => id == Id::ED25519,
        "x25519" => id == Id::X25519,
        _ => false,
    };
    if expected {
        Ok(())
    } else {
        Err(throw_dom_exception(
            ctx,
            "DataError",
            "The key does not match the requested algorithm",
        ))
    }
}

fn bundle_private(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    let (public, private) = match material {
        KeyMaterial::Private(key) => (
            key.public_key_to_der()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
            Some(
                key.private_key_to_pkcs8().map_err(|error| {
                    throw_dom_exception(ctx, "OperationError", &error.to_string())
                })?,
            ),
        ),
        KeyMaterial::Public(_) => {
            return Err(throw_dom_exception(
                ctx,
                "DataError",
                "A private key is required",
            ));
        }
    };
    bundle(&public, private.as_deref())
}

fn bundle_public(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    let public = material_public_der(ctx, material)?;
    bundle(&public, None)
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

fn material_public_der(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    match material {
        KeyMaterial::Private(key) => key
            .public_key_to_der()
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string())),
        KeyMaterial::Public(key) => key
            .public_key_to_der()
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string())),
    }
}

fn material_private_der(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    match material {
        KeyMaterial::Private(key) => key
            .private_key_to_pkcs8()
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string())),
        KeyMaterial::Public(_) => Err(throw_dom_exception(
            ctx,
            "InvalidAccessError",
            "The key is not private",
        )),
    }
}

fn material_raw_public(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    let public = material_public_key(ctx, material)?;
    match public.id() {
        id if id == Id::ED25519 || id == Id::X25519 => public
            .raw_public_key()
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string())),
        id if id == Id::EC => {
            let ec = public
                .ec_key()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            let mut bn_context = BigNumContext::new()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
            ec.public_key()
                .to_bytes(
                    ec.group(),
                    PointConversionForm::UNCOMPRESSED,
                    &mut bn_context,
                )
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
        }
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "Raw export is not supported for this algorithm",
        )),
    }
}

fn material_public_key(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<PKey<Public>> {
    let der = material_public_der(ctx, material)?;
    PKey::public_key_from_der(&der)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn material_jwk(ctx: &Ctx<'_>, material: &KeyMaterial) -> rquickjs::Result<Vec<u8>> {
    let jwk = match material {
        KeyMaterial::Private(key) => private_jwk(ctx, key)?,
        KeyMaterial::Public(key) => public_jwk(ctx, key)?,
    };
    serde_json::to_vec(&jwk).map_err(|error| Exception::throw_internal(ctx, &error.to_string()))
}

fn public_jwk<T: HasPublic>(ctx: &Ctx<'_>, key: &PKey<T>) -> rquickjs::Result<JsonValue> {
    match key.id() {
        id if id == Id::RSA => rsa_public_jwk(
            ctx,
            &key.rsa()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        ),
        id if id == Id::EC => ec_public_jwk(ctx, key),
        id if id == Id::ED25519 => okp_public_jwk(ctx, key, "Ed25519"),
        id if id == Id::X25519 => okp_public_jwk(ctx, key, "X25519"),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "JWK export is not supported for this algorithm",
        )),
    }
}

fn private_jwk(ctx: &Ctx<'_>, key: &PKey<Private>) -> rquickjs::Result<JsonValue> {
    match key.id() {
        id if id == Id::RSA => rsa_private_jwk(
            ctx,
            &key.rsa()
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        ),
        id if id == Id::EC => ec_private_jwk(ctx, key),
        id if id == Id::ED25519 => okp_private_jwk(ctx, key, "Ed25519"),
        id if id == Id::X25519 => okp_private_jwk(ctx, key, "X25519"),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "JWK export is not supported for this algorithm",
        )),
    }
}

fn rsa_public_jwk<T: HasPublic>(_ctx: &Ctx<'_>, rsa: &Rsa<T>) -> rquickjs::Result<JsonValue> {
    Ok(json!({
        "kty": "RSA",
        "n": URL_SAFE_NO_PAD.encode(rsa.n().to_vec()),
        "e": URL_SAFE_NO_PAD.encode(rsa.e().to_vec()),
    }))
}

fn rsa_private_jwk(ctx: &Ctx<'_>, rsa: &Rsa<Private>) -> rquickjs::Result<JsonValue> {
    Ok(json!({
        "kty": "RSA",
        "n": URL_SAFE_NO_PAD.encode(rsa.n().to_vec()),
        "e": URL_SAFE_NO_PAD.encode(rsa.e().to_vec()),
        "d": URL_SAFE_NO_PAD.encode(rsa.d().to_vec()),
        "p": URL_SAFE_NO_PAD.encode(rsa.p().ok_or_else(|| throw_dom_exception(ctx, "OperationError", "RSA private key is incomplete"))?.to_vec()),
        "q": URL_SAFE_NO_PAD.encode(rsa.q().ok_or_else(|| throw_dom_exception(ctx, "OperationError", "RSA private key is incomplete"))?.to_vec()),
        "dp": URL_SAFE_NO_PAD.encode(rsa.dmp1().ok_or_else(|| throw_dom_exception(ctx, "OperationError", "RSA private key is incomplete"))?.to_vec()),
        "dq": URL_SAFE_NO_PAD.encode(rsa.dmq1().ok_or_else(|| throw_dom_exception(ctx, "OperationError", "RSA private key is incomplete"))?.to_vec()),
        "qi": URL_SAFE_NO_PAD.encode(rsa.iqmp().ok_or_else(|| throw_dom_exception(ctx, "OperationError", "RSA private key is incomplete"))?.to_vec()),
    }))
}

fn ec_public_jwk<T: HasPublic>(ctx: &Ctx<'_>, key: &PKey<T>) -> rquickjs::Result<JsonValue> {
    let ec = key
        .ec_key()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let curve = curve_name(ctx, ec.group())?;
    let (x, y) = ec_coordinates(ctx, &ec)?;
    Ok(json!({
        "kty": "EC",
        "crv": curve,
        "x": URL_SAFE_NO_PAD.encode(x),
        "y": URL_SAFE_NO_PAD.encode(y),
    }))
}

fn ec_private_jwk(ctx: &Ctx<'_>, key: &PKey<Private>) -> rquickjs::Result<JsonValue> {
    let ec = key
        .ec_key()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let curve = curve_name(ctx, ec.group())?;
    let (x, y) = ec_coordinates(ctx, &ec)?;
    let size = curve_size(&curve).unwrap_or(0);
    Ok(json!({
        "kty": "EC",
        "crv": curve,
        "x": URL_SAFE_NO_PAD.encode(x),
        "y": URL_SAFE_NO_PAD.encode(y),
        "d": URL_SAFE_NO_PAD.encode(ec.private_key().to_vec_padded(size as i32).map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?),
    }))
}

fn ec_coordinates<T: HasPublic>(
    ctx: &Ctx<'_>,
    key: &EcKey<T>,
) -> rquickjs::Result<(Vec<u8>, Vec<u8>)> {
    let mut x = BigNum::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut y = BigNum::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let mut bn_context = BigNumContext::new()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    key.public_key()
        .affine_coordinates_gfp(key.group(), &mut x, &mut y, &mut bn_context)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let size = curve_name(ctx, key.group())
        .ok()
        .and_then(|curve| curve_size(&curve))
        .unwrap_or(0);
    Ok((
        x.to_vec_padded(size as i32)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
        y.to_vec_padded(size as i32)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
    ))
}

fn okp_public_jwk<T: HasPublic>(
    ctx: &Ctx<'_>,
    key: &PKey<T>,
    curve: &str,
) -> rquickjs::Result<JsonValue> {
    Ok(json!({
        "kty": "OKP",
        "crv": curve,
        "x": URL_SAFE_NO_PAD.encode(key.raw_public_key().map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?),
    }))
}

fn okp_private_jwk(ctx: &Ctx<'_>, key: &PKey<Private>, curve: &str) -> rquickjs::Result<JsonValue> {
    Ok(json!({
        "kty": "OKP",
        "crv": curve,
        "x": URL_SAFE_NO_PAD.encode(key.raw_public_key().map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?),
        "d": URL_SAFE_NO_PAD.encode(key.raw_private_key().map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?),
    }))
}

fn sign_key(ctx: &Ctx<'_>, message: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let kind = required_string(ctx, options, "kind")?;
    let key_bytes = message_key_bytes(ctx, options)?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let hash_name = required_string(ctx, options, "hash")?;
    let md = message_digest(ctx, &hash_name)?;
    match kind.as_str() {
        "rsa" => match material {
            KeyMaterial::Private(key) => rsa_sign(&key, md, false, 0, message, ctx),
            KeyMaterial::Public(_) => Err(throw_dom_exception(
                ctx,
                "InvalidAccessError",
                "A private key is required",
            )),
        },
        "rsa-pss" => match material {
            KeyMaterial::Private(key) => rsa_sign(
                &key,
                md,
                true,
                required_u32(ctx, options, "saltLength")?,
                message,
                ctx,
            ),
            KeyMaterial::Public(_) => Err(throw_dom_exception(
                ctx,
                "InvalidAccessError",
                "A private key is required",
            )),
        },
        "ecdsa" => match material {
            KeyMaterial::Private(key) => ecdsa_sign(ctx, &key, &hash_name, message),
            KeyMaterial::Public(_) => Err(throw_dom_exception(
                ctx,
                "InvalidAccessError",
                "A private key is required",
            )),
        },
        "ed25519" => match material {
            KeyMaterial::Private(key) => {
                let mut signer = Signer::new_without_digest(&key).map_err(|error| {
                    throw_dom_exception(ctx, "OperationError", &error.to_string())
                })?;
                signer
                    .sign_oneshot_to_vec(message)
                    .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
            }
            KeyMaterial::Public(_) => Err(throw_dom_exception(
                ctx,
                "InvalidAccessError",
                "A private key is required",
            )),
        },
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }
}

fn verify_key(ctx: &Ctx<'_>, signature: &[u8], options: &Object<'_>) -> rquickjs::Result<bool> {
    let kind = required_string(ctx, options, "kind")?;
    let message = required_option_bytes(ctx, options, "message")?;
    let key_bytes = message_key_bytes(ctx, options)?;
    let key = parse_key(ctx, &key_bytes, options)?;
    let hash_name = required_string(ctx, options, "hash")?;
    let md = message_digest(ctx, &hash_name)?;
    match kind.as_str() {
        "rsa" => match key {
            KeyMaterial::Private(key) => rsa_verify(&key, md, false, 0, &message, signature),
            KeyMaterial::Public(key) => rsa_verify(&key, md, false, 0, &message, signature),
        },
        "rsa-pss" => match key {
            KeyMaterial::Private(key) => rsa_verify(
                &key,
                md,
                true,
                required_u32(ctx, options, "saltLength")?,
                &message,
                signature,
            ),
            KeyMaterial::Public(key) => rsa_verify(
                &key,
                md,
                true,
                required_u32(ctx, options, "saltLength")?,
                &message,
                signature,
            ),
        },
        "ecdsa" => match key {
            KeyMaterial::Private(key) => ecdsa_verify(ctx, &key, &hash_name, &message, signature),
            KeyMaterial::Public(key) => ecdsa_verify(ctx, &key, &hash_name, &message, signature),
        },
        "ed25519" => match key {
            KeyMaterial::Private(key) => ed_verify(&key, &message, signature),
            KeyMaterial::Public(key) => ed_verify(&key, &message, signature),
        },
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }
}

fn rsa_sign<T: HasPrivate>(
    key: &PKey<T>,
    md: MessageDigest,
    pss: bool,
    salt_length: u32,
    message: &[u8],
    ctx: &Ctx<'_>,
) -> rquickjs::Result<Vec<u8>> {
    let mut signer = Signer::new(md, key)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    if pss {
        signer
            .set_rsa_padding(Padding::PKCS1_PSS)
            .and_then(|_| signer.set_rsa_pss_saltlen(RsaPssSaltlen::custom(salt_length as i32)))
            .and_then(|_| signer.set_rsa_mgf1_md(md))
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    } else {
        signer
            .set_rsa_padding(Padding::PKCS1)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    }
    signer
        .sign_oneshot_to_vec(message)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn rsa_verify<T: HasPublic>(
    key: &PKey<T>,
    md: MessageDigest,
    pss: bool,
    salt_length: u32,
    message: &[u8],
    signature: &[u8],
) -> rquickjs::Result<bool> {
    let mut verifier = match Verifier::new(md, key) {
        Ok(verifier) => verifier,
        Err(_) => return Ok(false),
    };
    if pss {
        if verifier.set_rsa_padding(Padding::PKCS1_PSS).is_err()
            || verifier
                .set_rsa_pss_saltlen(RsaPssSaltlen::custom(salt_length as i32))
                .is_err()
            || verifier.set_rsa_mgf1_md(md).is_err()
        {
            return Ok(false);
        }
    } else if verifier.set_rsa_padding(Padding::PKCS1).is_err() {
        return Ok(false);
    }
    Ok(verifier.verify_oneshot(signature, message).unwrap_or(false))
}

fn ecdsa_sign(
    ctx: &Ctx<'_>,
    key: &PKey<Private>,
    hash_name: &str,
    message: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let digest = hash(message_digest(ctx, hash_name)?, message)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let ec = key
        .ec_key()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let signature = EcdsaSig::sign(digest.as_ref(), &ec)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let size = curve_name(ctx, ec.group())
        .ok()
        .and_then(|curve| curve_size(&curve))
        .ok_or_else(|| throw_dom_exception(ctx, "OperationError", "Unsupported elliptic curve"))?;
    let mut output = signature
        .r()
        .to_vec_padded(size as i32)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.extend_from_slice(
        &signature
            .s()
            .to_vec_padded(size as i32)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?,
    );
    Ok(output)
}

fn ecdsa_verify<T: HasPublic>(
    ctx: &Ctx<'_>,
    key: &PKey<T>,
    hash_name: &str,
    message: &[u8],
    signature: &[u8],
) -> rquickjs::Result<bool> {
    let ec = key
        .ec_key()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let size = curve_name(ctx, ec.group())
        .ok()
        .and_then(|curve| curve_size(&curve))
        .ok_or_else(|| throw_dom_exception(ctx, "OperationError", "Unsupported elliptic curve"))?;
    if signature.len() != size * 2 {
        return Ok(false);
    }
    let r = BigNum::from_slice(&signature[..size])
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let s = BigNum::from_slice(&signature[size..])
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let signature = EcdsaSig::from_private_components(r, s)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let digest = hash(message_digest(ctx, hash_name)?, message)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    Ok(signature.verify(digest.as_ref(), &ec).unwrap_or(false))
}

fn ed_verify<T: HasPublic>(
    key: &PKey<T>,
    message: &[u8],
    signature: &[u8],
) -> rquickjs::Result<bool> {
    if signature.len() != 64 {
        return Ok(false);
    }
    let mut verifier = match Verifier::new_without_digest(key) {
        Ok(verifier) => verifier,
        Err(_) => return Ok(false),
    };
    Ok(verifier.verify_oneshot(signature, message).unwrap_or(false))
}

fn encrypt_key(ctx: &Ctx<'_>, message: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    if required_string(ctx, options, "kind")? != "rsa-oaep" {
        return Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        ));
    }
    let key_bytes = message_key_bytes(ctx, options)?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let hash = message_digest(ctx, &required_string(ctx, options, "hash")?)?;
    let label = optional_bytes(ctx, options, "label")?;
    match material {
        KeyMaterial::Private(key) => rsa_encrypt(ctx, &key, hash, &label, message),
        KeyMaterial::Public(key) => rsa_encrypt(ctx, &key, hash, &label, message),
    }
}

fn decrypt_key(
    ctx: &Ctx<'_>,
    ciphertext: &[u8],
    options: &Object<'_>,
) -> rquickjs::Result<Vec<u8>> {
    if required_string(ctx, options, "kind")? != "rsa-oaep" {
        return Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        ));
    }
    let key_bytes = message_key_bytes(ctx, options)?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let hash = message_digest(ctx, &required_string(ctx, options, "hash")?)?;
    let label = optional_bytes(ctx, options, "label")?;
    match material {
        KeyMaterial::Private(key) => rsa_decrypt(ctx, &key, hash, &label, ciphertext),
        KeyMaterial::Public(_) => Err(throw_dom_exception(
            ctx,
            "InvalidAccessError",
            "A private key is required",
        )),
    }
}

fn rsa_encrypt<T: HasPublic>(
    ctx: &Ctx<'_>,
    key: &PKey<T>,
    hash: MessageDigest,
    label: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let mut encrypter = Encrypter::new(key)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    encrypter
        .set_rsa_padding(Padding::PKCS1_OAEP)
        .and_then(|_| encrypter.set_rsa_oaep_md(hash))
        .and_then(|_| encrypter.set_rsa_mgf1_md(hash))
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    if !label.is_empty() {
        encrypter
            .set_rsa_oaep_label(label)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    }
    let mut output =
        vec![
            0;
            encrypter
                .encrypt_len(input)
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?
        ];
    let length = encrypter
        .encrypt(input, &mut output)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.truncate(length);
    Ok(output)
}

fn rsa_decrypt(
    ctx: &Ctx<'_>,
    key: &PKey<Private>,
    hash: MessageDigest,
    label: &[u8],
    input: &[u8],
) -> rquickjs::Result<Vec<u8>> {
    let mut decrypter = Decrypter::new(key)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    decrypter
        .set_rsa_padding(Padding::PKCS1_OAEP)
        .and_then(|_| decrypter.set_rsa_oaep_md(hash))
        .and_then(|_| decrypter.set_rsa_mgf1_md(hash))
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    if !label.is_empty() {
        decrypter
            .set_rsa_oaep_label(label)
            .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    }
    let mut output =
        vec![
            0;
            decrypter
                .decrypt_len(input)
                .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?
        ];
    let length = decrypter
        .decrypt(input, &mut output)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    output.truncate(length);
    Ok(output)
}

fn derive_key(ctx: &Ctx<'_>, _input: &[u8], options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    let key_bytes = message_key_bytes(ctx, options)?;
    let material = parse_key(ctx, &key_bytes, options)?;
    let peer = required_option_bytes(ctx, options, "peer")?;
    let peer = PKey::public_key_from_der(&peer)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    let KeyMaterial::Private(private) = material else {
        return Err(throw_dom_exception(
            ctx,
            "InvalidAccessError",
            "A private key is required",
        ));
    };
    let mut deriver = Deriver::new(&private)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    deriver
        .set_peer(&peer)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))?;
    deriver
        .derive_to_vec()
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn ec_group(ctx: &Ctx<'_>, curve: &str) -> rquickjs::Result<EcGroup> {
    let nid = match curve {
        "P-256" => Nid::X9_62_PRIME256V1,
        "P-384" => Nid::SECP384R1,
        "P-521" => Nid::SECP521R1,
        _ => {
            return Err(throw_dom_exception(
                ctx,
                "NotSupportedError",
                "The requested elliptic curve is not supported",
            ));
        }
    };
    EcGroup::from_curve_name(nid)
        .map_err(|error| throw_dom_exception(ctx, "OperationError", &error.to_string()))
}

fn curve_name(ctx: &Ctx<'_>, group: &openssl::ec::EcGroupRef) -> rquickjs::Result<String> {
    match group.curve_name() {
        Some(nid) if nid == Nid::X9_62_PRIME256V1 => Ok("P-256".to_owned()),
        Some(nid) if nid == Nid::SECP384R1 => Ok("P-384".to_owned()),
        Some(nid) if nid == Nid::SECP521R1 => Ok("P-521".to_owned()),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The elliptic curve is not supported",
        )),
    }
}

fn curve_size(curve: &str) -> Option<usize> {
    match curve {
        "P-256" => Some(32),
        "P-384" => Some(48),
        "P-521" => Some(66),
        _ => None,
    }
}

fn message_digest(ctx: &Ctx<'_>, algorithm: &str) -> rquickjs::Result<MessageDigest> {
    match algorithm {
        "MD5" => Ok(MessageDigest::md5()),
        "SHA-1" => Ok(MessageDigest::sha1()),
        "SHA-256" => Ok(MessageDigest::sha256()),
        "SHA-384" => Ok(MessageDigest::sha384()),
        "SHA-512" => Ok(MessageDigest::sha512()),
        _ => Err(throw_dom_exception(
            ctx,
            "NotSupportedError",
            "The requested cryptographic algorithm is not supported",
        )),
    }
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

fn required_option_bytes(
    ctx: &Ctx<'_>,
    options: &Object<'_>,
    name: &str,
) -> rquickjs::Result<Vec<u8>> {
    let value = option_bytes(ctx, options, name)?;
    match value {
        Some(value) => Ok(value),
        None => Err(Exception::throw_type(ctx, &format!("{name} is required"))),
    }
}

fn message_key_bytes(ctx: &Ctx<'_>, options: &Object<'_>) -> rquickjs::Result<Vec<u8>> {
    required_bytes(ctx, options, "key")
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
            let bytes = value
                .as_bytes()
                .map(ToOwned::to_owned)
                .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))?;
            Ok(bytes)
        })
        .transpose()
}

fn bytes<'js>(ctx: &Ctx<'js>, input: TypedArray<'js, u8>) -> rquickjs::Result<Vec<u8>> {
    input
        .as_bytes()
        .map(ToOwned::to_owned)
        .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))
}

fn jwk_string(ctx: &Ctx<'_>, jwk: &JsonValue, name: &str) -> rquickjs::Result<String> {
    jwk.get(name)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            throw_dom_exception(ctx, "DataError", &format!("JWK field {name} is required"))
        })
}

fn jwk_bytes(ctx: &Ctx<'_>, jwk: &JsonValue, name: &str) -> rquickjs::Result<Vec<u8>> {
    let value = jwk_string(ctx, jwk, name)?;
    URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))
}

fn jwk_big_num(ctx: &Ctx<'_>, jwk: &JsonValue, name: &str) -> rquickjs::Result<BigNum> {
    let value = jwk_bytes(ctx, jwk, name)?;
    BigNum::from_slice(&value)
        .map_err(|error| throw_dom_exception(ctx, "DataError", &error.to_string()))
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
