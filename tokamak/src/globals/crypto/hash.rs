use hmac::digest::InvalidLength;
use hmac::digest::{self, Digest, DynDigest};
use hmac::{EagerHash, Hmac, KeyInit, Mac};
use md5::Md5;
use sha1::Sha1;

/// Digest algorithms named as `WebCrypto` spells them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Hash {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

/// Runs `$body` with `$d` bound to the digest type selected by `$hash`.
macro_rules! with_digest {
    ($hash:expr, $d:ident => $body:expr) => {
        match $hash {
            $crate::globals::crypto::hash::Hash::Md5 => {
                type $d = ::md5::Md5;
                $body
            }
            $crate::globals::crypto::hash::Hash::Sha1 => {
                type $d = ::sha1::Sha1;
                $body
            }
            $crate::globals::crypto::hash::Hash::Sha224 => {
                type $d = ::sha2::Sha224;
                $body
            }
            $crate::globals::crypto::hash::Hash::Sha256 => {
                type $d = ::sha2::Sha256;
                $body
            }
            $crate::globals::crypto::hash::Hash::Sha384 => {
                type $d = ::sha2::Sha384;
                $body
            }
            $crate::globals::crypto::hash::Hash::Sha512 => {
                type $d = ::sha2::Sha512;
                $body
            }
        }
    };
}
pub(super) use with_digest;

impl Hash {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name {
            "MD5" => Some(Self::Md5),
            "SHA-1" => Some(Self::Sha1),
            "SHA-224" => Some(Self::Sha224),
            "SHA-256" => Some(Self::Sha256),
            "SHA-384" => Some(Self::Sha384),
            "SHA-512" => Some(Self::Sha512),
            _ => None,
        }
    }

    pub(super) fn digest(self, data: &[u8]) -> Vec<u8> {
        with_digest!(self, D => D::digest(data).to_vec())
    }

    pub(super) fn hmac(self, key: &[u8], data: &[u8]) -> Result<Vec<u8>, InvalidLength> {
        let mut mac = self.mac(key)?;
        mac.update(data);
        Ok(mac.finalize())
    }

    pub(super) fn pbkdf2(self, password: &[u8], salt: &[u8], iterations: u32, output: &mut [u8]) {
        with_digest!(self, D => pbkdf2::pbkdf2_hmac::<D>(password, salt, iterations, output));
    }

    pub(super) fn hkdf(
        self,
        salt: Option<&[u8]>,
        key: &[u8],
        info: &[u8],
        output: &mut [u8],
    ) -> Result<(), hkdf::InvalidLength> {
        with_digest!(self, D => hkdf::Hkdf::<D>::new(salt, key).expand(info, output))
    }

    pub(super) fn hasher(self) -> Box<dyn DynDigest> {
        with_digest!(self, D => Box::new(D::new()))
    }

    pub(super) fn mac(self, key: &[u8]) -> Result<Box<dyn MacState>, InvalidLength> {
        with_digest!(self, D => Ok(Box::new(<Hmac<D> as KeyInit>::new_from_slice(key)?)))
    }
}

/// An HMAC in progress, independent of its digest type.
pub(super) trait MacState {
    fn update(&mut self, data: &[u8]);
    fn finalize(self: Box<Self>) -> Vec<u8>;
}

impl<D: EagerHash> MacState for Hmac<D> {
    fn update(&mut self, data: &[u8]) {
        Mac::update(self, data);
    }

    fn finalize(self: Box<Self>) -> Vec<u8> {
        Mac::finalize(*self).into_bytes().to_vec()
    }
}

/// The concatenated MD5 and SHA-1 digest used by TLS 1.1 signatures.
#[derive(Clone, Default)]
pub(super) struct Md5Sha1 {
    md5: Md5,
    sha1: Sha1,
}

impl DynDigest for Md5Sha1 {
    fn update(&mut self, data: &[u8]) {
        Digest::update(&mut self.md5, data);
        Digest::update(&mut self.sha1, data);
    }

    fn finalize_into(self, buf: &mut [u8]) -> Result<(), digest::InvalidBufferSize> {
        if buf.len() != 36 {
            return Err(digest::InvalidBufferSize);
        }
        buf[..16].copy_from_slice(&self.md5.finalize());
        buf[16..].copy_from_slice(&self.sha1.finalize());
        Ok(())
    }

    fn finalize_into_reset(&mut self, out: &mut [u8]) -> Result<(), digest::InvalidBufferSize> {
        self.clone().finalize_into(out)?;
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        Digest::reset(&mut self.md5);
        Digest::reset(&mut self.sha1);
    }

    fn output_size(&self) -> usize {
        36
    }

    fn box_clone(&self) -> Box<dyn DynDigest> {
        Box::new(self.clone())
    }
}

/// HMAC (RFC 2104) over a boxed digest, for digests without a block-level API.
pub(super) struct DynHmac {
    inner: Box<dyn DynDigest>,
    outer: Box<dyn DynDigest>,
}

impl DynHmac {
    pub(super) fn new(mut hasher: Box<dyn DynDigest>, block_size: usize, key: &[u8]) -> Self {
        let mut padded = if key.len() > block_size {
            hasher.update(key);
            hasher.finalize_reset().to_vec()
        } else {
            key.to_vec()
        };
        padded.resize(block_size, 0);
        let mut outer = hasher.box_clone();
        let mut inner = hasher;
        inner.update(&padded.iter().map(|byte| byte ^ 0x36).collect::<Vec<u8>>());
        outer.update(&padded.iter().map(|byte| byte ^ 0x5c).collect::<Vec<u8>>());
        Self { inner, outer }
    }
}

impl MacState for DynHmac {
    fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    fn finalize(mut self: Box<Self>) -> Vec<u8> {
        let inner = self.inner.finalize_reset();
        self.outer.update(&inner);
        self.outer.finalize_reset().to_vec()
    }
}
