use std::collections::BTreeMap;
use std::ffi::CStr;
use std::ptr::{self, NonNull};

use brotlic_sys as b;

use super::node::{CodecError, Step};

pub(crate) enum Brotli {
    Encoder(NonNull<b::BrotliEncoderState>),
    Decoder(NonNull<b::BrotliDecoderState>),
}

// SAFETY: A Brotli instance is exclusively owned and holds no thread affinity;
// Brotli's public API allows moving an instance between threads.
unsafe impl Send for Brotli {}

impl Brotli {
    pub(crate) fn new(encode: bool, params: &BTreeMap<u32, i32>) -> Result<Self, CodecError> {
        let failure = || {
            CodecError::new(
                "ERR_ZLIB_INITIALIZATION_FAILED",
                "Brotli initialization failed",
            )
        };
        // SAFETY: Null allocators select Brotli's allocator. This owner destroys
        // the returned instance once, including when parameter setup fails.
        let codec = unsafe {
            if encode {
                Self::Encoder(
                    NonNull::new(b::BrotliEncoderCreateInstance(None, None, ptr::null_mut()))
                        .ok_or_else(failure)?,
                )
            } else {
                Self::Decoder(
                    NonNull::new(b::BrotliDecoderCreateInstance(None, None, ptr::null_mut()))
                        .ok_or_else(failure)?,
                )
            }
        };
        for (&key, &value) in params {
            // SAFETY: The instance is initialized and unused. Brotli validates
            // parameter numbers and values before accepting them.
            let accepted = unsafe {
                match &codec {
                    Self::Encoder(state) => b::BrotliEncoderSetParameter(
                        state.as_ptr(),
                        key.cast_signed(),
                        value.cast_unsigned(),
                    ),
                    Self::Decoder(state) => b::BrotliDecoderSetParameter(
                        state.as_ptr(),
                        key.cast_signed(),
                        value.cast_unsigned(),
                    ),
                }
            };
            if accepted == 0 {
                return Err(failure());
            }
        }
        Ok(codec)
    }

    pub(crate) fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: i32,
    ) -> Result<Step, CodecError> {
        if !(0..=3).contains(&flush) {
            return Err(CodecError::new(
                "Z_STREAM_ERROR",
                "Invalid Brotli operation",
            ));
        }
        let (mut available_in, mut available_out) = (input.len(), output.len());
        let (mut next_in, mut next_out) = (input.as_ptr(), output.as_mut_ptr());
        // SAFETY: The instance is exclusively borrowed, input and output do not
        // overlap, and their lengths bound every access. Brotli does not retain
        // either caller-owned slice after this call.
        let ended = unsafe {
            match self {
                Self::Encoder(state) => {
                    let accepted = b::BrotliEncoderCompressStream(
                        state.as_ptr(),
                        flush,
                        &raw mut available_in,
                        &raw mut next_in,
                        &raw mut available_out,
                        &raw mut next_out,
                        ptr::null_mut(),
                    );
                    if accepted == 0 {
                        return Err(CodecError::new(
                            "ERR_BROTLI_COMPRESSION_FAILED",
                            "Brotli compression failed",
                        ));
                    }
                    b::BrotliEncoderIsFinished(state.as_ptr()) != 0
                }
                Self::Decoder(state) => {
                    match b::BrotliDecoderDecompressStream(
                        state.as_ptr(),
                        &raw mut available_in,
                        &raw mut next_in,
                        &raw mut available_out,
                        &raw mut next_out,
                        ptr::null_mut(),
                    ) {
                        b::BrotliDecoderResult_BROTLI_DECODER_RESULT_SUCCESS => true,
                        b::BrotliDecoderResult_BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT => false,
                        b::BrotliDecoderResult_BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT
                            if flush != 2 =>
                        {
                            false
                        }
                        b::BrotliDecoderResult_BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT => {
                            return Err(CodecError::new("Z_BUF_ERROR", "unexpected end of file"));
                        }
                        _ => {
                            let code = b::BrotliDecoderGetErrorCode(state.as_ptr());
                            let name =
                                CStr::from_ptr(b::BrotliDecoderErrorString(code)).to_string_lossy();
                            // Brotli 1.1 returns the short suffix. Workerd uses
                            // the full enum spelling used by newer Brotli builds.
                            let prefix = match code {
                                -16..=-1 => "_ERROR_FORMAT_",
                                -30..=-21 => "_ERROR_ALLOC_",
                                _ => "_ERROR_",
                            };
                            return Err(CodecError::new(&format!("ERR_{prefix}{name}"), &name));
                        }
                    }
                }
            }
        };
        Ok(Step {
            consumed: input.len() - available_in,
            written: output.len() - available_out,
            ended,
        })
    }
}

impl Drop for Brotli {
    fn drop(&mut self) {
        // SAFETY: This is the unique owner of an initialized instance.
        unsafe {
            match self {
                Self::Encoder(state) => b::BrotliEncoderDestroyInstance(state.as_ptr()),
                Self::Decoder(state) => b::BrotliDecoderDestroyInstance(state.as_ptr()),
            }
        }
    }
}
