//! RSA signatures, encryption and the raw operations node:crypto exposes.

use rsa::traits::PublicKeyParts;
use rsa::{BoxedUint, Oaep, Pkcs1v15Sign, Pss, RsaPrivateKey, RsaPublicKey};

use super::hash::{Hash, with_digest};
use super::keys::{KeyError, integer};
use super::rng;

type Result<T> = std::result::Result<T, KeyError>;

pub(super) fn generate(modulus_bits: usize, exponent: &[u8]) -> Result<RsaPrivateKey> {
    Ok(RsaPrivateKey::new_with_exp(
        &mut rng(),
        modulus_bits,
        integer(exponent),
    )?)
}

pub(super) fn sign_pkcs1(key: &RsaPrivateKey, hash: Hash, message: &[u8]) -> Result<Vec<u8>> {
    let digest = hash.digest(message);
    Ok(with_digest!(hash, D => key.sign(Pkcs1v15Sign::new::<D>(), &digest)?))
}

pub(super) fn verify_pkcs1(
    key: &RsaPublicKey,
    hash: Hash,
    message: &[u8],
    signature: &[u8],
) -> bool {
    let digest = hash.digest(message);
    with_digest!(hash, D => key.verify(Pkcs1v15Sign::new::<D>(), &digest, signature).is_ok())
}

pub(super) fn sign_pss(
    key: &RsaPrivateKey,
    hash: Hash,
    salt_length: usize,
    message: &[u8],
) -> Result<Vec<u8>> {
    let digest = hash.digest(message);
    Ok(with_digest!(hash, D => {
        key.sign_with_rng(&mut rng(), Pss::<D>::new_with_salt(salt_length), &digest)?
    }))
}

pub(super) fn verify_pss(
    key: &RsaPublicKey,
    hash: Hash,
    salt_length: usize,
    message: &[u8],
    signature: &[u8],
) -> bool {
    let digest = hash.digest(message);
    with_digest!(hash, D => {
        key.verify(Pss::<D>::new_with_salt(salt_length), &digest, signature).is_ok()
    })
}

pub(super) fn encrypt_oaep(
    key: &RsaPublicKey,
    hash: Hash,
    label: &[u8],
    message: &[u8],
) -> Result<Vec<u8>> {
    Ok(with_digest!(hash, D => key.encrypt(&mut rng(), oaep::<D>(label), message)?))
}

pub(super) fn decrypt_oaep(
    key: &RsaPrivateKey,
    hash: Hash,
    label: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    Ok(with_digest!(hash, D => key.decrypt_blinded(&mut rng(), oaep::<D>(label), ciphertext)?))
}

fn oaep<D: hmac::digest::Digest + hmac::digest::FixedOutputReset>(label: &[u8]) -> Oaep<D> {
    if label.is_empty() {
        Oaep::<D>::new()
    } else {
        Oaep::<D>::new_with_label(label)
    }
}

/// node:crypto padding constants.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Padding {
    Pkcs1,
    None,
}

impl Padding {
    pub(super) fn parse(value: Option<u32>) -> Option<Self> {
        match value {
            None | Some(1) => Some(Self::Pkcs1),
            Some(3) => Some(Self::None),
            Some(_) => None,
        }
    }
}

/// `privateEncrypt`: a raw private-key operation over PKCS#1 type 1 padding.
pub(super) fn private_encrypt(
    key: &RsaPrivateKey,
    padding: Padding,
    input: &[u8],
) -> Result<Vec<u8>> {
    match padding {
        Padding::Pkcs1 => Ok(key.sign(Pkcs1v15Sign::new_unprefixed(), input)?),
        Padding::None => {
            let size = key.size();
            if input.len() != size {
                return Err(KeyError::Operation("data must be the key size".to_owned()));
            }
            let value = BoxedUint::from_be_slice(input, key.n_bits_precision())?;
            let output = rsa::hazmat::rsa_decrypt_and_check(key, Some(&mut rng()), &value)?;
            Ok(fixed_width(&output.to_be_bytes(), size))
        }
    }
}

/// `publicDecrypt`: the raw public-key operation, stripping type 1 padding.
pub(super) fn public_decrypt(
    key: &RsaPublicKey,
    padding: Padding,
    input: &[u8],
) -> Result<Vec<u8>> {
    let size = key.size();
    if input.len() > size {
        return Err(KeyError::Operation(
            "data too large for key size".to_owned(),
        ));
    }
    let value = BoxedUint::from_be_slice(&fixed_width(input, size), key.n_bits_precision())?;
    let output = fixed_width(&rsa::hazmat::rsa_encrypt(key, &value)?.to_be_bytes(), size);
    match padding {
        Padding::None => Ok(output),
        Padding::Pkcs1 => strip_type1_padding(&output),
    }
}

fn strip_type1_padding(block: &[u8]) -> Result<Vec<u8>> {
    let invalid = || KeyError::Operation("padding check failed".to_owned());
    let [0, 1, rest @ ..] = block else {
        return Err(invalid());
    };
    let separator = rest
        .iter()
        .position(|byte| *byte != 0xff)
        .ok_or_else(invalid)?;
    if rest[separator] != 0 || separator < 8 {
        return Err(invalid());
    }
    Ok(rest[separator + 1..].to_vec())
}

fn fixed_width(value: &[u8], size: usize) -> Vec<u8> {
    let trimmed = &value[value.len().saturating_sub(size)..];
    let mut output = vec![0; size - trimmed.len()];
    output.extend_from_slice(trimmed);
    output
}
