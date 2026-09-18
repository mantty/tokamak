//! AES modes as incremental streams shared by `WebCrypto` and `node:crypto`.

use aes::cipher::consts::U16;
use aes::cipher::{
    Block, BlockCipherDecrypt, BlockCipherEncrypt, BlockModeDecrypt, BlockModeEncrypt,
    BlockSizeUser, KeyInit, KeyIvInit, StreamCipher,
};
use aes::{Aes128, Aes192, Aes256};
use aes_kw::AesKw;
use ghash::GHash;
use ghash::universal_hash::UniversalHash;
use subtle::ConstantTimeEq;

const BLOCK: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Gcm,
    Cbc,
    Ctr,
}

/// A failed AES operation, described as `node:crypto` reports it.
#[derive(Debug)]
pub(super) struct Failure(pub(super) &'static str);

pub(super) struct Finished {
    pub(super) output: Vec<u8>,
    pub(super) tag: Option<Vec<u8>>,
}

/// An encryption or decryption in progress.
pub(super) trait Stream {
    fn update(&mut self, input: &[u8]) -> Result<Vec<u8>, Failure>;
    fn set_padding(&mut self, enabled: bool);
    fn set_aad(&mut self, data: &[u8]) -> Result<(), Failure>;
    fn set_tag(&mut self, tag: &[u8]) -> Result<(), Failure>;
    fn finish(self: Box<Self>) -> Result<Finished, Failure>;
}

/// Runs `$body` with `$c` bound to the AES type matching the key length.
macro_rules! with_aes {
    ($key:expr, $c:ident => $body:expr) => {
        match $key.len() {
            16 => {
                type $c = Aes128;
                $body
            }
            24 => {
                type $c = Aes192;
                $body
            }
            32 => {
                type $c = Aes256;
                $body
            }
            _ => return Err(Failure("Invalid key length")),
        }
    };
}

pub(super) fn valid_key(key: &[u8]) -> bool {
    matches!(key.len(), 16 | 24 | 32)
}

pub(super) fn valid_tag_length(bytes: usize) -> bool {
    matches!(bytes, 4 | 8 | 12 | 13 | 14 | 15 | 16)
}

pub(super) fn stream(
    mode: Mode,
    encrypt: bool,
    key: &[u8],
    iv: &[u8],
    tag_length: usize,
) -> Result<Box<dyn Stream>, Failure> {
    match mode {
        Mode::Gcm => {
            with_aes!(key, C => Ok(Box::new(Gcm::<C>::new(key, iv, encrypt, tag_length)?)))
        }
        Mode::Cbc if encrypt => with_aes!(key, C => Ok(Box::new(CbcEncrypt::<C>::new(key, iv)?))),
        Mode::Cbc => with_aes!(key, C => Ok(Box::new(CbcDecrypt::<C>::new(key, iv)?))),
        Mode::Ctr => with_aes!(key, C => Ok(Box::new(Ctr::<C>::new(key, iv)?))),
    }
}

pub(super) fn wrap_key(key: &[u8], input: &[u8]) -> Result<Vec<u8>, Failure> {
    with_aes!(key, C => {
        let kek = AesKw::<C>::new_from_slice(key).map_err(|_| Failure("Invalid key length"))?;
        let mut output = vec![0; input.len() + 8];
        let length = kek
            .wrap_key(input, &mut output)
            .map_err(|_| Failure("AES-KW wrapping failed"))?
            .len();
        output.truncate(length);
        Ok(output)
    })
}

pub(super) fn unwrap_key(key: &[u8], input: &[u8]) -> Result<Vec<u8>, Failure> {
    with_aes!(key, C => {
        let kek = AesKw::<C>::new_from_slice(key).map_err(|_| Failure("Invalid key length"))?;
        let mut output = vec![0; input.len().saturating_sub(8)];
        let length = kek
            .unwrap_key(input, &mut output)
            .map_err(|_| Failure("AES-KW integrity check failed"))?
            .len();
        output.truncate(length);
        Ok(output)
    })
}

fn invalid_iv<E>(_: E) -> Failure {
    Failure("Invalid initialization vector")
}

fn blocks<C: BlockSizeUser>(data: &[u8]) -> Vec<Block<C>> {
    data.chunks_exact(BLOCK)
        .map(|chunk| {
            let mut block = Block::<C>::default();
            block.copy_from_slice(chunk);
            block
        })
        .collect()
}

fn flatten<C: BlockSizeUser>(blocks: &[Block<C>]) -> Vec<u8> {
    blocks.concat()
}

struct CbcEncrypt<C: BlockCipherEncrypt> {
    cipher: cbc::Encryptor<C>,
    pending: Vec<u8>,
    padding: bool,
}

impl<C: BlockCipherEncrypt + KeyInit> CbcEncrypt<C> {
    fn new(key: &[u8], iv: &[u8]) -> Result<Self, Failure> {
        Ok(Self {
            cipher: cbc::Encryptor::<C>::new_from_slices(key, iv).map_err(invalid_iv)?,
            pending: Vec::new(),
            padding: true,
        })
    }

    fn encrypt(&mut self, data: &[u8]) -> Vec<u8> {
        let mut blocks = blocks::<C>(data);
        self.cipher.encrypt_blocks(&mut blocks);
        flatten::<C>(&blocks)
    }
}

impl<C: BlockCipherEncrypt + KeyInit> Stream for CbcEncrypt<C> {
    fn update(&mut self, input: &[u8]) -> Result<Vec<u8>, Failure> {
        self.pending.extend_from_slice(input);
        let whole = self.pending.len() / BLOCK * BLOCK;
        let ready: Vec<u8> = self.pending.drain(..whole).collect();
        Ok(self.encrypt(&ready))
    }

    fn set_padding(&mut self, enabled: bool) {
        self.padding = enabled;
    }

    fn set_aad(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support AAD"))
    }

    fn set_tag(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support auth tags"))
    }

    fn finish(mut self: Box<Self>) -> Result<Finished, Failure> {
        if !self.padding {
            if !self.pending.is_empty() {
                return Err(Failure("data not multiple of block length"));
            }
            return Ok(Finished {
                output: Vec::new(),
                tag: None,
            });
        }
        let fill = BLOCK - self.pending.len();
        let mut last = std::mem::take(&mut self.pending);
        last.resize(BLOCK, u8::try_from(fill).unwrap_or(16));
        Ok(Finished {
            output: self.encrypt(&last),
            tag: None,
        })
    }
}

struct CbcDecrypt<C: BlockCipherDecrypt> {
    cipher: cbc::Decryptor<C>,
    pending: Vec<u8>,
    padding: bool,
}

impl<C: BlockCipherDecrypt + KeyInit> CbcDecrypt<C> {
    fn new(key: &[u8], iv: &[u8]) -> Result<Self, Failure> {
        Ok(Self {
            cipher: cbc::Decryptor::<C>::new_from_slices(key, iv).map_err(invalid_iv)?,
            pending: Vec::new(),
            padding: true,
        })
    }

    fn decrypt(&mut self, data: &[u8]) -> Vec<u8> {
        let mut blocks = blocks::<C>(data);
        self.cipher.decrypt_blocks(&mut blocks);
        flatten::<C>(&blocks)
    }
}

impl<C: BlockCipherDecrypt + KeyInit> Stream for CbcDecrypt<C> {
    fn update(&mut self, input: &[u8]) -> Result<Vec<u8>, Failure> {
        self.pending.extend_from_slice(input);
        // With padding, the last whole block is held back until finish strips it.
        let held = usize::from(self.padding && !self.pending.is_empty());
        let whole = (self.pending.len() - held) / BLOCK * BLOCK;
        let ready: Vec<u8> = self.pending.drain(..whole).collect();
        Ok(self.decrypt(&ready))
    }

    fn set_padding(&mut self, enabled: bool) {
        self.padding = enabled;
    }

    fn set_aad(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support AAD"))
    }

    fn set_tag(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support auth tags"))
    }

    fn finish(mut self: Box<Self>) -> Result<Finished, Failure> {
        if !self.padding {
            if !self.pending.is_empty() {
                return Err(Failure("wrong final block length"));
            }
            return Ok(Finished {
                output: Vec::new(),
                tag: None,
            });
        }
        if self.pending.len() != BLOCK {
            return Err(Failure("wrong final block length"));
        }
        let last = std::mem::take(&mut self.pending);
        let mut output = self.decrypt(&last);
        let fill = usize::from(output.last().copied().unwrap_or(0));
        if fill == 0
            || fill > BLOCK
            || output[BLOCK - fill..]
                .iter()
                .any(|byte| usize::from(*byte) != fill)
        {
            return Err(Failure("bad decrypt"));
        }
        output.truncate(BLOCK - fill);
        Ok(Finished { output, tag: None })
    }
}

struct Ctr<C: BlockCipherEncrypt<BlockSize = U16>> {
    cipher: ctr::Ctr128BE<C>,
}

impl<C: BlockCipherEncrypt<BlockSize = U16> + KeyInit> Ctr<C> {
    fn new(key: &[u8], iv: &[u8]) -> Result<Self, Failure> {
        Ok(Self {
            cipher: ctr::Ctr128BE::<C>::new_from_slices(key, iv).map_err(invalid_iv)?,
        })
    }
}

impl<C: BlockCipherEncrypt<BlockSize = U16> + KeyInit> Stream for Ctr<C> {
    fn update(&mut self, input: &[u8]) -> Result<Vec<u8>, Failure> {
        let mut output = input.to_vec();
        self.cipher.apply_keystream(&mut output);
        Ok(output)
    }

    fn set_padding(&mut self, _: bool) {}

    fn set_aad(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support AAD"))
    }

    fn set_tag(&mut self, _: &[u8]) -> Result<(), Failure> {
        Err(Failure("Cipher does not support auth tags"))
    }

    fn finish(self: Box<Self>) -> Result<Finished, Failure> {
        Ok(Finished {
            output: Vec::new(),
            tag: None,
        })
    }
}

/// GCM (NIST SP 800-38D) over a 32-bit counter with GHASH over the AAD and
/// ciphertext, accumulated as data arrives.
struct Gcm<C: BlockCipherEncrypt<BlockSize = U16>> {
    cipher: ctr::Ctr32BE<C>,
    ghash: GHash,
    mask: [u8; BLOCK],
    aad: Vec<u8>,
    aad_done: bool,
    pending: Vec<u8>,
    text_length: u64,
    encrypt: bool,
    tag_length: usize,
    expected_tag: Option<Vec<u8>>,
}

impl<C: BlockCipherEncrypt<BlockSize = U16> + KeyInit> Gcm<C> {
    fn new(key: &[u8], iv: &[u8], encrypt: bool, tag_length: usize) -> Result<Self, Failure> {
        if iv.is_empty() {
            return Err(Failure("Invalid initialization vector"));
        }
        if !valid_tag_length(tag_length) {
            return Err(Failure("Invalid authentication tag length"));
        }
        let block = C::new_from_slice(key).map_err(|_| Failure("Invalid key length"))?;
        let mut subkey = Block::<C>::default();
        block.encrypt_block(&mut subkey);
        let ghash = GHash::new(&subkey);
        let counter = initial_counter(&ghash, iv);
        let mut cipher = ctr::Ctr32BE::<C>::new_from_slices(key, &counter).map_err(invalid_iv)?;
        let mut mask = [0; BLOCK];
        cipher.apply_keystream(&mut mask);
        Ok(Self {
            cipher,
            ghash,
            mask,
            aad: Vec::new(),
            aad_done: false,
            pending: Vec::new(),
            text_length: 0,
            encrypt,
            tag_length,
            expected_tag: None,
        })
    }

    fn absorb_aad(&mut self) {
        if !self.aad_done {
            self.ghash.update_padded(&self.aad);
            self.aad_done = true;
        }
    }

    fn absorb_text(&mut self, ciphertext: &[u8]) {
        self.pending.extend_from_slice(ciphertext);
        let whole = self.pending.len() / BLOCK * BLOCK;
        let ready: Vec<u8> = self.pending.drain(..whole).collect();
        self.ghash.update(&blocks::<GHash>(&ready));
        self.text_length += ciphertext.len() as u64;
    }

    fn tag(mut self) -> Vec<u8> {
        self.ghash.update_padded(&self.pending);
        let mut lengths = [0; BLOCK];
        lengths[..8].copy_from_slice(&(self.aad.len() as u64 * 8).to_be_bytes());
        lengths[8..].copy_from_slice(&(self.text_length * 8).to_be_bytes());
        self.ghash.update(&blocks::<GHash>(&lengths));
        let mut tag = self.ghash.finalize();
        for (byte, mask) in tag.iter_mut().zip(self.mask) {
            *byte ^= mask;
        }
        tag[..self.tag_length].to_vec()
    }
}

fn initial_counter(ghash: &GHash, iv: &[u8]) -> [u8; BLOCK] {
    let mut counter = [0; BLOCK];
    if iv.len() == 12 {
        counter[..12].copy_from_slice(iv);
        counter[15] = 1;
        return counter;
    }
    let mut hash = ghash.clone();
    hash.update_padded(iv);
    let mut lengths = [0; BLOCK];
    lengths[8..].copy_from_slice(&(iv.len() as u64 * 8).to_be_bytes());
    hash.update(&blocks::<GHash>(&lengths));
    counter.copy_from_slice(&hash.finalize());
    counter
}

impl<C: BlockCipherEncrypt<BlockSize = U16> + KeyInit> Stream for Gcm<C> {
    fn update(&mut self, input: &[u8]) -> Result<Vec<u8>, Failure> {
        self.absorb_aad();
        let mut output = input.to_vec();
        self.cipher.apply_keystream(&mut output);
        if self.encrypt {
            self.absorb_text(&output);
        } else {
            self.absorb_text(input);
        }
        Ok(output)
    }

    fn set_padding(&mut self, _: bool) {}

    fn set_aad(&mut self, data: &[u8]) -> Result<(), Failure> {
        if self.aad_done {
            return Err(Failure("AAD must be set before any data"));
        }
        self.aad.extend_from_slice(data);
        Ok(())
    }

    fn set_tag(&mut self, tag: &[u8]) -> Result<(), Failure> {
        if self.encrypt || !valid_tag_length(tag.len()) {
            return Err(Failure("Invalid authentication tag length"));
        }
        self.expected_tag = Some(tag.to_vec());
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<Finished, Failure> {
        self.absorb_aad();
        if self.encrypt {
            let tag = self.tag();
            return Ok(Finished {
                output: Vec::new(),
                tag: Some(tag),
            });
        }
        let expected = self
            .expected_tag
            .take()
            .ok_or(Failure("Unsupported state or unable to authenticate data"))?;
        self.tag_length = expected.len();
        if self.tag().ct_eq(&expected).into() {
            Ok(Finished {
                output: Vec::new(),
                tag: None,
            })
        } else {
            Err(Failure("Unsupported state or unable to authenticate data"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16))
            .collect::<Result<_, _>>()
            .unwrap_or_default()
    }

    struct Case<'a> {
        mode: Mode,
        encrypt: bool,
        key: &'a [u8],
        iv: &'a [u8],
        aad: &'a [u8],
        tag: Option<&'a [u8]>,
    }

    fn run(case: &Case<'_>, input: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), Failure> {
        let tag_length = case.tag.map_or(16, <[u8]>::len);
        let mut stream = stream(case.mode, case.encrypt, case.key, case.iv, tag_length)?;
        if !case.aad.is_empty() {
            stream.set_aad(case.aad)?;
        }
        if let Some(tag) = case.tag {
            stream.set_tag(tag)?;
        }
        let mut output = Vec::new();
        for chunk in input.chunks(7) {
            output.extend(stream.update(chunk)?);
        }
        let finished = stream.finish()?;
        output.extend(finished.output);
        Ok((output, finished.tag))
    }

    // NIST GCM test case 4: AES-128, 96-bit IV, AAD, partial final block.
    #[test]
    fn gcm_matches_the_nist_vector() -> Result<(), Failure> {
        let key = hex("feffe9928665731c6d6a8f9467308308");
        let iv = hex("cafebabefacedbaddecaf888");
        let plaintext = hex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        );
        let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let expected = hex(
            "42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa051ba30b396a0aac973d58e091",
        );
        let tag = hex("5bc94fbc3221a5db94fae95ae7121a47");
        let (output, produced) = run(
            &Case {
                mode: Mode::Gcm,
                encrypt: true,
                key: &key,
                iv: &iv,
                aad: &aad,
                tag: None,
            },
            &plaintext,
        )?;
        assert_eq!(output, expected);
        assert_eq!(produced.as_deref(), Some(tag.as_slice()));
        let (decrypted, _) = run(
            &Case {
                mode: Mode::Gcm,
                encrypt: false,
                key: &key,
                iv: &iv,
                aad: &aad,
                tag: Some(&tag),
            },
            &expected,
        )?;
        assert_eq!(decrypted, plaintext);
        let mut wrong = tag.clone();
        wrong[0] ^= 1;
        assert!(
            run(
                &Case {
                    mode: Mode::Gcm,
                    encrypt: false,
                    key: &key,
                    iv: &iv,
                    aad: &aad,
                    tag: Some(&wrong)
                },
                &expected
            )
            .is_err()
        );
        Ok(())
    }

    // NIST GCM test case 6: 480-bit IV, so the counter comes from GHASH.
    #[test]
    fn gcm_hashes_long_ivs() -> Result<(), Failure> {
        let key = hex("feffe9928665731c6d6a8f9467308308");
        let iv = hex(
            "9313225df88406e555909c5aff5269aa6a7a9538534f7da1e4c303d2a318a728c3c0c95156809539fcf0e2429a6b525416aedbf5a0de6a57a637b39b",
        );
        let plaintext = hex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
        );
        let aad = hex("feedfacedeadbeeffeedfacedeadbeefabaddad2");
        let expected = hex(
            "8ce24998625615b603a033aca13fb894be9112a5c3a211a8ba262a3cca7e2ca701e4a9a4fba43c90ccdcb281d48c7c6fd62875d2aca417034c34aee5",
        );
        let tag = hex("619cc5aefffe0bfa462af43c1699d050");
        let (output, produced) = run(
            &Case {
                mode: Mode::Gcm,
                encrypt: true,
                key: &key,
                iv: &iv,
                aad: &aad,
                tag: None,
            },
            &plaintext,
        )?;
        assert_eq!(output, expected);
        assert_eq!(produced.as_deref(), Some(tag.as_slice()));
        Ok(())
    }

    #[test]
    fn cbc_pads_and_strips_pkcs7() -> Result<(), Failure> {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let plaintext = b"The quick brown fox jumps over the lazy dog";
        let (ciphertext, _) = run(
            &Case {
                mode: Mode::Cbc,
                encrypt: true,
                key: &key,
                iv: &iv,
                aad: &[],
                tag: None,
            },
            plaintext,
        )?;
        assert_eq!(ciphertext.len(), 48);
        let (decrypted, _) = run(
            &Case {
                mode: Mode::Cbc,
                encrypt: false,
                key: &key,
                iv: &iv,
                aad: &[],
                tag: None,
            },
            &ciphertext,
        )?;
        assert_eq!(decrypted, plaintext);
        let mut corrupted = ciphertext.clone();
        corrupted[47] ^= 1;
        assert!(
            run(
                &Case {
                    mode: Mode::Cbc,
                    encrypt: false,
                    key: &key,
                    iv: &iv,
                    aad: &[],
                    tag: None
                },
                &corrupted
            )
            .is_err()
        );
        Ok(())
    }

    // NIST SP 800-38A F.5.1 CTR-AES128.Encrypt, first two blocks.
    #[test]
    fn ctr_matches_the_nist_vector() -> Result<(), Failure> {
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
        let plaintext = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        let expected = hex("874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff");
        let (output, _) = run(
            &Case {
                mode: Mode::Ctr,
                encrypt: true,
                key: &key,
                iv: &iv,
                aad: &[],
                tag: None,
            },
            &plaintext,
        )?;
        assert_eq!(output, expected);
        Ok(())
    }

    // RFC 3394 section 4.1.
    #[test]
    fn key_wrap_matches_rfc_3394() -> Result<(), Failure> {
        let kek = hex("000102030405060708090A0B0C0D0E0F");
        let key = hex("00112233445566778899AABBCCDDEEFF");
        let wrapped = wrap_key(&kek, &key)?;
        assert_eq!(
            wrapped,
            hex("1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5")
        );
        assert_eq!(unwrap_key(&kek, &wrapped)?, key);
        Ok(())
    }
}
