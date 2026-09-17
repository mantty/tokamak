//! NIST curve operations dispatched over the three supported curves, plus the
//! Edwards and Montgomery curve signatures and agreements.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::{AssociatedOid, ObjectIdentifier};
use x25519_dalek::StaticSecret;

use super::keys::KeyError;

type Result<T> = std::result::Result<T, KeyError>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Curve {
    P256,
    P384,
    P521,
}

impl Curve {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "P-256" => Some(Self::P256),
            "P-384" => Some(Self::P384),
            "P-521" => Some(Self::P521),
            _ => None,
        }
    }

    /// The OpenSSL curve names node:crypto's ECDH class accepts.
    pub(super) fn parse_node(name: &str) -> Option<Self> {
        match name {
            "prime256v1" => Some(Self::P256),
            "secp384r1" => Some(Self::P384),
            "secp521r1" => Some(Self::P521),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::P256 => "P-256",
            Self::P384 => "P-384",
            Self::P521 => "P-521",
        }
    }

    /// Field element size in bytes.
    pub(super) fn size(self) -> usize {
        match self {
            Self::P256 => 32,
            Self::P384 => 48,
            Self::P521 => 66,
        }
    }

    pub(super) fn from_oid(oid: ObjectIdentifier) -> Result<Self> {
        match oid {
            p256::NistP256::OID => Ok(Self::P256),
            p384::NistP384::OID => Ok(Self::P384),
            p521::NistP521::OID => Ok(Self::P521),
            _ => Err(KeyError::Data(
                "The elliptic curve is not supported".to_owned(),
            )),
        }
    }
}

#[derive(Clone)]
pub(super) enum EcSecret {
    P256(p256::SecretKey),
    P384(p384::SecretKey),
    P521(p521::SecretKey),
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum EcPublic {
    P256(p256::PublicKey),
    P384(p384::PublicKey),
    P521(p521::PublicKey),
}

/// Runs `$body` with `$inner` bound to the wrapped key and `$c` to its curve crate.
macro_rules! with_curve {
    ($value:expr, $variant:ident, $inner:ident, $c:ident => $body:expr) => {
        match $value {
            $variant::P256($inner) => {
                use p256 as $c;
                $body
            }
            $variant::P384($inner) => {
                use p384 as $c;
                $body
            }
            $variant::P521($inner) => {
                use p521 as $c;
                $body
            }
        }
    };
}

/// Builds the `$variant` for `$curve`, running `$body` with `$c` bound to its crate.
macro_rules! for_curve {
    ($curve:expr, $variant:ident, $c:ident => $body:expr) => {
        match $curve {
            Curve::P256 => {
                use p256 as $c;
                $variant::P256($body)
            }
            Curve::P384 => {
                use p384 as $c;
                $variant::P384($body)
            }
            Curve::P521 => {
                use p521 as $c;
                $variant::P521($body)
            }
        }
    };
}

pub(super) fn generate(curve: Curve) -> EcSecret {
    for_curve!(curve, EcSecret, c => {
        use c::elliptic_curve::Generate;
        c::SecretKey::generate()
    })
}

pub(super) fn curve_of(key: &EcPublic) -> Curve {
    match key {
        EcPublic::P256(_) => Curve::P256,
        EcPublic::P384(_) => Curve::P384,
        EcPublic::P521(_) => Curve::P521,
    }
}

pub(super) fn public_of(key: &EcSecret) -> EcPublic {
    match key {
        EcSecret::P256(inner) => EcPublic::P256(inner.public_key()),
        EcSecret::P384(inner) => EcPublic::P384(inner.public_key()),
        EcSecret::P521(inner) => EcPublic::P521(inner.public_key()),
    }
}

/// A private scalar given as big-endian bytes, padded to the field size.
pub(super) fn secret_from_scalar(curve: Curve, scalar: &[u8]) -> Result<EcSecret> {
    if scalar.len() > curve.size() {
        return Err(KeyError::Data("The private key is too long".to_owned()));
    }
    let mut padded = vec![0; curve.size() - scalar.len()];
    padded.extend_from_slice(scalar);
    Ok(for_curve!(curve, EcSecret, c => c::SecretKey::from_slice(&padded)?))
}

pub(super) fn secret_bytes(key: &EcSecret) -> Vec<u8> {
    match key {
        EcSecret::P256(inner) => inner.to_bytes().to_vec(),
        EcSecret::P384(inner) => inner.to_bytes().to_vec(),
        EcSecret::P521(inner) => inner.to_bytes().to_vec(),
    }
}

pub(super) fn secret_from_pkcs8(curve: Curve, der: &[u8]) -> Result<EcSecret> {
    Ok(for_curve!(curve, EcSecret, c => {
        use c::pkcs8::DecodePrivateKey;
        c::SecretKey::from_pkcs8_der(der)?
    }))
}

pub(super) fn secret_to_pkcs8(key: &EcSecret) -> Result<Vec<u8>> {
    with_curve!(key, EcSecret, inner, c => {
        use c::pkcs8::EncodePrivateKey;
        Ok(inner.to_pkcs8_der()?.as_bytes().to_vec())
    })
}

pub(super) fn public_to_spki(key: &EcPublic) -> Result<Vec<u8>> {
    with_curve!(key, EcPublic, inner, c => {
        use c::pkcs8::EncodePublicKey;
        Ok(inner.to_public_key_der()?.into_vec())
    })
}

/// Decodes a SEC1 point in compressed or uncompressed form.
pub(super) fn decode_point(curve: Curve, bytes: &[u8]) -> Result<EcPublic> {
    Ok(for_curve!(curve, EcPublic, c => c::PublicKey::from_sec1_bytes(bytes)?))
}

pub(super) fn encode_point(key: &EcPublic, compressed: bool) -> Vec<u8> {
    with_curve!(key, EcPublic, inner, c => {
        use c::elliptic_curve::sec1::ToSec1Point;
        inner.to_sec1_point(compressed).as_bytes().to_vec()
    })
}

/// Affine coordinates at the field size, as JWK requires.
pub(super) fn coordinates(key: &EcPublic) -> (Vec<u8>, Vec<u8>) {
    let point = encode_point(key, false);
    let size = curve_of(key).size();
    (point[1..=size].to_vec(), point[size + 1..].to_vec())
}

pub(super) fn sign(key: &EcSecret, prehash: &[u8]) -> Result<Vec<u8>> {
    with_curve!(key, EcSecret, inner, c => {
        use c::ecdsa::signature::hazmat::PrehashSigner;
        let signer = c::ecdsa::SigningKey::from(inner);
        let signature: c::ecdsa::Signature = signer.sign_prehash(prehash)?;
        Ok(signature.to_bytes().to_vec())
    })
}

pub(super) fn verify(key: &EcPublic, prehash: &[u8], signature: &[u8]) -> bool {
    with_curve!(key, EcPublic, inner, c => {
        use c::ecdsa::signature::hazmat::PrehashVerifier;
        let Ok(signature) = c::ecdsa::Signature::from_slice(signature) else {
            return false;
        };
        c::ecdsa::VerifyingKey::from(inner)
            .verify_prehash(prehash, &signature)
            .is_ok()
    })
}

pub(super) fn agree(key: &EcSecret, peer: &EcPublic) -> Result<Vec<u8>> {
    let mismatch = || KeyError::Operation("The peer key is on a different curve".to_owned());
    match (key, peer) {
        (EcSecret::P256(key), EcPublic::P256(peer)) => {
            Ok(shared_secret::<p256::NistP256>(key, peer))
        }
        (EcSecret::P384(key), EcPublic::P384(peer)) => {
            Ok(shared_secret::<p384::NistP384>(key, peer))
        }
        (EcSecret::P521(key), EcPublic::P521(peer)) => {
            Ok(shared_secret::<p521::NistP521>(key, peer))
        }
        _ => Err(mismatch()),
    }
}

fn shared_secret<C: p256::elliptic_curve::CurveArithmetic>(
    key: &p256::elliptic_curve::SecretKey<C>,
    peer: &p256::elliptic_curve::PublicKey<C>,
) -> Vec<u8> {
    p256::elliptic_curve::ecdh::diffie_hellman(key.to_nonzero_scalar(), peer.as_affine())
        .raw_secret_bytes()
        .to_vec()
}

pub(super) fn ed25519_sign(key: &SigningKey, message: &[u8]) -> Vec<u8> {
    key.sign(message).to_bytes().to_vec()
}

pub(super) fn ed25519_verify(key: &VerifyingKey, message: &[u8], signature: &[u8]) -> bool {
    Signature::from_slice(signature).is_ok_and(|signature| key.verify(message, &signature).is_ok())
}

pub(super) fn x25519_agree(key: &StaticSecret, peer: &x25519_dalek::PublicKey) -> Vec<u8> {
    key.diffie_hellman(peer).as_bytes().to_vec()
}
