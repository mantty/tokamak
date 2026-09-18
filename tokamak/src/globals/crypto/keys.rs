//! Asymmetric key material and its DER, raw and JWK encodings.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use der::asn1::{BitStringRef, OctetStringRef};
use der::{Decode, Encode};
use ed25519_dalek::{SigningKey, VerifyingKey};
use pkcs8::{DecodePrivateKey, EncodePrivateKey, ObjectIdentifier, PrivateKeyInfoRef};
use rsa::traits::{PrivateKeyParts, PublicKeyParts};
use rsa::{BoxedUint, RsaPrivateKey, RsaPublicKey};
use serde_json::{Value as JsonValue, json};
use spki::{AlgorithmIdentifierRef, DecodePublicKey, EncodePublicKey, SubjectPublicKeyInfoRef};
use x25519_dalek::StaticSecret;

use super::curves::{self, Curve, EcPublic, EcSecret};

const RSA_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const EC_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.2.1");
const ED25519_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.112");
const X25519_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.110");

/// A key could not be decoded or encoded (`Data`), or an operation on a
/// well-formed key failed (`Operation`), matching the `DOMException` names.
#[derive(Debug)]
pub(super) enum KeyError {
    Data(String),
    Operation(String),
}

impl KeyError {
    pub(super) fn message(&self) -> &str {
        match self {
            Self::Data(message) | Self::Operation(message) => message,
        }
    }
}

impl<E: std::fmt::Display> From<E> for KeyError {
    fn from(error: E) -> Self {
        Self::Data(error.to_string())
    }
}

type Result<T> = std::result::Result<T, KeyError>;

/// Key families as named by the JavaScript layer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Rsa,
    Ec,
    Ed25519,
    X25519,
}

impl Kind {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "rsa" | "rsa-pss" | "rsa-oaep" => Some(Self::Rsa),
            "ec" | "ecdsa" => Some(Self::Ec),
            "ed25519" => Some(Self::Ed25519),
            "x25519" => Some(Self::X25519),
            _ => None,
        }
    }
}

pub(super) enum PrivateKey {
    Rsa(RsaPrivateKey),
    Ec(EcSecret),
    Ed25519(SigningKey),
    X25519(StaticSecret),
}

pub(super) enum PublicKey {
    Rsa(RsaPublicKey),
    Ec(EcPublic),
    Ed25519(VerifyingKey),
    X25519(x25519_dalek::PublicKey),
}

pub(super) enum KeyMaterial {
    Private(PrivateKey),
    Public(PublicKey),
}

impl KeyMaterial {
    pub(super) fn kind(&self) -> Kind {
        match self {
            Self::Private(key) => key.kind(),
            Self::Public(key) => key.kind(),
        }
    }

    pub(super) fn public(&self) -> PublicKey {
        match self {
            Self::Private(key) => key.public(),
            Self::Public(key) => key.clone(),
        }
    }
}

impl PrivateKey {
    pub(super) fn kind(&self) -> Kind {
        match self {
            Self::Rsa(_) => Kind::Rsa,
            Self::Ec(_) => Kind::Ec,
            Self::Ed25519(_) => Kind::Ed25519,
            Self::X25519(_) => Kind::X25519,
        }
    }

    pub(super) fn public(&self) -> PublicKey {
        match self {
            Self::Rsa(key) => PublicKey::Rsa(key.to_public_key()),
            Self::Ec(key) => PublicKey::Ec(curves::public_of(key)),
            Self::Ed25519(key) => PublicKey::Ed25519(key.verifying_key()),
            Self::X25519(key) => PublicKey::X25519(x25519_dalek::PublicKey::from(key)),
        }
    }

    pub(super) fn from_pkcs8(der: &[u8]) -> Result<Self> {
        let info = PrivateKeyInfoRef::try_from(der)?;
        let key = match info.algorithm.oid {
            RSA_OID => Self::Rsa(RsaPrivateKey::from_pkcs8_der(der)?),
            EC_OID => {
                let curve = Curve::from_oid(info.algorithm.parameters_oid()?)?;
                Self::Ec(curves::secret_from_pkcs8(curve, der)?)
            }
            ED25519_OID => Self::Ed25519(SigningKey::try_from(info)?),
            X25519_OID => {
                let inner = <&OctetStringRef as Decode>::from_der(info.private_key.as_bytes())?;
                Self::X25519(StaticSecret::from(<[u8; 32]>::try_from(inner.as_bytes())?))
            }
            _ => {
                return Err(KeyError::Data(
                    "Unsupported private key algorithm".to_owned(),
                ));
            }
        };
        Ok(key)
    }

    pub(super) fn to_pkcs8(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Rsa(key) => key.to_pkcs8_der()?.as_bytes().to_vec(),
            Self::Ec(key) => curves::secret_to_pkcs8(key)?,
            Self::Ed25519(key) => okp_pkcs8(ED25519_OID, key.as_bytes())?,
            Self::X25519(key) => okp_pkcs8(X25519_OID, key.as_bytes())?,
        })
    }

    pub(super) fn to_jwk(&self) -> Result<JsonValue> {
        Ok(match self {
            Self::Rsa(key) => rsa_private_jwk(key)?,
            Self::Ec(key) => {
                let mut jwk = ec_public_jwk(&curves::public_of(key));
                jwk["d"] = json!(URL_SAFE_NO_PAD.encode(curves::secret_bytes(key)));
                jwk
            }
            Self::Ed25519(key) => okp_jwk(
                "Ed25519",
                key.verifying_key().as_bytes(),
                Some(&key.to_bytes()),
            ),
            Self::X25519(key) => okp_jwk(
                "X25519",
                x25519_dalek::PublicKey::from(key).as_bytes(),
                Some(&key.to_bytes()),
            ),
        })
    }
}

impl PublicKey {
    pub(super) fn kind(&self) -> Kind {
        match self {
            Self::Rsa(_) => Kind::Rsa,
            Self::Ec(_) => Kind::Ec,
            Self::Ed25519(_) => Kind::Ed25519,
            Self::X25519(_) => Kind::X25519,
        }
    }

    pub(super) fn from_spki(der: &[u8]) -> Result<Self> {
        let info = SubjectPublicKeyInfoRef::try_from(der)?;
        let raw = || {
            info.subject_public_key
                .as_bytes()
                .ok_or_else(|| KeyError::Data("Malformed public key".to_owned()))
        };
        let key = match info.algorithm.oid {
            RSA_OID => Self::Rsa(RsaPublicKey::from_public_key_der(der)?),
            EC_OID => {
                let curve = Curve::from_oid(info.algorithm.parameters_oid()?)?;
                Self::Ec(curves::decode_point(curve, raw()?)?)
            }
            ED25519_OID => Self::Ed25519(VerifyingKey::try_from(info)?),
            X25519_OID => {
                Self::X25519(x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(raw()?)?))
            }
            _ => {
                return Err(KeyError::Data(
                    "Unsupported public key algorithm".to_owned(),
                ));
            }
        };
        Ok(key)
    }

    pub(super) fn to_spki(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Rsa(key) => key.to_public_key_der()?.into_vec(),
            Self::Ec(key) => curves::public_to_spki(key)?,
            Self::Ed25519(key) => key.to_public_key_der()?.into_vec(),
            Self::X25519(key) => SubjectPublicKeyInfoRef {
                algorithm: AlgorithmIdentifierRef {
                    oid: X25519_OID,
                    parameters: None,
                },
                subject_public_key: BitStringRef::from_bytes(key.as_bytes())?,
            }
            .to_der()?,
        })
    }

    /// The raw encoding `WebCrypto` defines: an uncompressed point or the OKP bytes.
    pub(super) fn to_raw(&self) -> Option<Vec<u8>> {
        match self {
            Self::Rsa(_) => None,
            Self::Ec(key) => Some(curves::encode_point(key, false)),
            Self::Ed25519(key) => Some(key.as_bytes().to_vec()),
            Self::X25519(key) => Some(key.as_bytes().to_vec()),
        }
    }

    pub(super) fn from_raw(kind: Kind, curve: Option<Curve>, raw: &[u8]) -> Result<Self> {
        Ok(match kind {
            Kind::Ec => {
                let curve = curve.ok_or_else(|| KeyError::Data("curve is required".to_owned()))?;
                Self::Ec(curves::decode_point(curve, raw)?)
            }
            Kind::Ed25519 => Self::Ed25519(VerifyingKey::from_bytes(&<[u8; 32]>::try_from(raw)?)?),
            Kind::X25519 => Self::X25519(x25519_dalek::PublicKey::from(<[u8; 32]>::try_from(raw)?)),
            Kind::Rsa => {
                return Err(KeyError::Data(
                    "Raw import is not supported for RSA".to_owned(),
                ));
            }
        })
    }

    pub(super) fn to_jwk(&self) -> JsonValue {
        match self {
            Self::Rsa(key) => rsa_public_jwk(key),
            Self::Ec(key) => ec_public_jwk(key),
            Self::Ed25519(key) => okp_jwk("Ed25519", key.as_bytes(), None),
            Self::X25519(key) => okp_jwk("X25519", key.as_bytes(), None),
        }
    }
}

impl Clone for PublicKey {
    fn clone(&self) -> Self {
        match self {
            Self::Rsa(key) => Self::Rsa(key.clone()),
            Self::Ec(key) => Self::Ec(key.clone()),
            Self::Ed25519(key) => Self::Ed25519(*key),
            Self::X25519(key) => Self::X25519(*key),
        }
    }
}

pub(super) fn from_jwk(kind: Kind, jwk: &JsonValue) -> Result<KeyMaterial> {
    let key_type = jwk
        .get("kty")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    match (kind, key_type) {
        (Kind::Rsa, "RSA") => rsa_from_jwk(jwk),
        (Kind::Ec, "EC") => ec_from_jwk(jwk),
        (Kind::Ed25519, "OKP") => {
            let (public, secret) = okp_from_jwk(jwk, "Ed25519")?;
            Ok(match secret {
                Some(secret) => {
                    KeyMaterial::Private(PrivateKey::Ed25519(SigningKey::from_bytes(&secret)))
                }
                None => KeyMaterial::Public(PublicKey::Ed25519(VerifyingKey::from_bytes(&public)?)),
            })
        }
        (Kind::X25519, "OKP") => {
            let (public, secret) = okp_from_jwk(jwk, "X25519")?;
            Ok(match secret {
                Some(secret) => {
                    KeyMaterial::Private(PrivateKey::X25519(StaticSecret::from(secret)))
                }
                None => {
                    KeyMaterial::Public(PublicKey::X25519(x25519_dalek::PublicKey::from(public)))
                }
            })
        }
        _ => Err(KeyError::Data(
            "The JWK does not match the requested algorithm".to_owned(),
        )),
    }
}

#[allow(clippy::many_single_char_names)] // Names follow the RSA JWK fields.
fn rsa_from_jwk(jwk: &JsonValue) -> Result<KeyMaterial> {
    let n = integer(&jwk_bytes(jwk, "n")?);
    let e = integer(&jwk_bytes(jwk, "e")?);
    if jwk.get("d").is_none() {
        return Ok(KeyMaterial::Public(PublicKey::Rsa(RsaPublicKey::new(
            n, e,
        )?)));
    }
    let d = integer(&jwk_bytes(jwk, "d")?);
    let p = integer(&jwk_bytes(jwk, "p")?);
    let q = integer(&jwk_bytes(jwk, "q")?);
    for field in ["dp", "dq", "qi"] {
        jwk_bytes(jwk, field)?;
    }
    let key = RsaPrivateKey::from_components(n, e, d, vec![p, q])?;
    Ok(KeyMaterial::Private(PrivateKey::Rsa(key)))
}

fn ec_from_jwk(jwk: &JsonValue) -> Result<KeyMaterial> {
    let curve = Curve::parse(&jwk_string(jwk, "crv")?)
        .ok_or_else(|| KeyError::Data("The JWK curve is not supported".to_owned()))?;
    let x = fixed_width(&jwk_bytes(jwk, "x")?, curve.size())?;
    let y = fixed_width(&jwk_bytes(jwk, "y")?, curve.size())?;
    let mut point = vec![4];
    point.extend(x);
    point.extend(y);
    let public = curves::decode_point(curve, &point)
        .map_err(|error| KeyError::Operation(error.message().to_owned()))?;
    if jwk.get("d").is_none() {
        return Ok(KeyMaterial::Public(PublicKey::Ec(public)));
    }
    let secret = curves::secret_from_scalar(curve, &jwk_bytes(jwk, "d")?)?;
    if curves::public_of(&secret) != public {
        return Err(KeyError::Operation(
            "The JWK private key does not match its public key".to_owned(),
        ));
    }
    Ok(KeyMaterial::Private(PrivateKey::Ec(secret)))
}

fn okp_from_jwk(jwk: &JsonValue, curve: &str) -> Result<([u8; 32], Option<[u8; 32]>)> {
    if jwk_string(jwk, "crv")? != curve {
        return Err(KeyError::Data(
            "The JWK curve does not match the requested algorithm".to_owned(),
        ));
    }
    let public = <[u8; 32]>::try_from(jwk_bytes(jwk, "x")?.as_slice())?;
    let secret = match jwk.get("d") {
        Some(_) => Some(<[u8; 32]>::try_from(jwk_bytes(jwk, "d")?.as_slice())?),
        None => None,
    };
    Ok((public, secret))
}

fn rsa_public_jwk(key: &RsaPublicKey) -> JsonValue {
    json!({
        "kty": "RSA",
        "n": URL_SAFE_NO_PAD.encode(bytes(key.n())),
        "e": URL_SAFE_NO_PAD.encode(bytes(key.e())),
    })
}

fn rsa_private_jwk(key: &RsaPrivateKey) -> Result<JsonValue> {
    let incomplete = || KeyError::Operation("RSA private key is incomplete".to_owned());
    let [p, q] = key.primes() else {
        return Err(incomplete());
    };
    Ok(json!({
        "kty": "RSA",
        "n": URL_SAFE_NO_PAD.encode(bytes(key.n())),
        "e": URL_SAFE_NO_PAD.encode(bytes(key.e())),
        "d": URL_SAFE_NO_PAD.encode(bytes(key.d())),
        "p": URL_SAFE_NO_PAD.encode(bytes(p)),
        "q": URL_SAFE_NO_PAD.encode(bytes(q)),
        "dp": URL_SAFE_NO_PAD.encode(bytes(key.dp().ok_or_else(incomplete)?)),
        "dq": URL_SAFE_NO_PAD.encode(bytes(key.dq().ok_or_else(incomplete)?)),
        "qi": URL_SAFE_NO_PAD.encode(bytes(&key.qinv().ok_or_else(incomplete)?.retrieve())),
    }))
}

fn ec_public_jwk(key: &EcPublic) -> JsonValue {
    let (x, y) = curves::coordinates(key);
    json!({
        "kty": "EC",
        "crv": curves::curve_of(key).name(),
        "x": URL_SAFE_NO_PAD.encode(x),
        "y": URL_SAFE_NO_PAD.encode(y),
    })
}

fn okp_jwk(curve: &str, public: &[u8], secret: Option<&[u8]>) -> JsonValue {
    let mut jwk = json!({
        "kty": "OKP",
        "crv": curve,
        "x": URL_SAFE_NO_PAD.encode(public),
    });
    if let Some(secret) = secret {
        jwk["d"] = json!(URL_SAFE_NO_PAD.encode(secret));
    }
    jwk
}

/// Big-endian bytes without leading zeros, as JWK and node:crypto encode integers.
pub(super) fn bytes(value: &BoxedUint) -> Vec<u8> {
    value.to_be_bytes_trimmed_vartime().into_vec()
}

pub(super) fn integer(bytes: &[u8]) -> BoxedUint {
    if bytes.is_empty() {
        BoxedUint::zero()
    } else {
        BoxedUint::from_be_slice_vartime(bytes)
    }
}

fn fixed_width(value: &[u8], size: usize) -> Result<Vec<u8>> {
    if value.len() > size {
        return Err(KeyError::Data("The JWK coordinate is too long".to_owned()));
    }
    let mut padded = vec![0; size - value.len()];
    padded.extend_from_slice(value);
    Ok(padded)
}

fn jwk_string(jwk: &JsonValue, name: &str) -> Result<String> {
    jwk.get(name)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| KeyError::Data(format!("JWK field {name} is required")))
}

fn jwk_bytes(jwk: &JsonValue, name: &str) -> Result<Vec<u8>> {
    Ok(URL_SAFE_NO_PAD.decode(jwk_string(jwk, name)?)?)
}

/// PKCS#8 v1 for an Edwards or Montgomery key: the bare secret as a nested
/// OCTET STRING, the form OpenSSL and workerd emit.
fn okp_pkcs8(oid: ObjectIdentifier, secret: &[u8]) -> Result<Vec<u8>> {
    let inner = OctetStringRef::new(secret)?.to_der()?;
    let algorithm = AlgorithmIdentifierRef {
        oid,
        parameters: None,
    };
    Ok(PrivateKeyInfoRef::new(algorithm, OctetStringRef::new(&inner)?).to_der()?)
}
