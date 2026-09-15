//! Owned access to zlib-rs's stable zlib ABI. It exposes dictionary, strategy,
//! window, reset and flush controls which flate2's safe facade does not expose.

use std::ffi::CStr;
use std::ptr;

use libz_rs_sys as z;

use super::node::{CodecError, Step};

// The zlib ABI struct contains a fixed number of pointers and scalar counters.
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
const STREAM_SIZE: i32 = size_of::<z::z_stream>() as i32;

pub(super) struct Zlib {
    stream: Box<z::z_stream>,
    mode: u8,
    dictionary: Vec<u8>,
    ended: bool,
    gzip: Option<bool>,
}

impl Zlib {
    pub(super) fn new(
        mode: u8,
        window: i32,
        level: i32,
        memory: i32,
        strategy: i32,
        dictionary: Vec<u8>,
    ) -> Result<Self, CodecError> {
        let mut stream = Box::new(z::z_stream::default());
        let window = match mode {
            3 | 4 => window + 16,
            5 | 6 => -window,
            7 => window + 32,
            _ => window,
        };
        // SAFETY: The zero-initialized stream is kept at a stable address until
        // its matching End call. The ABI version and size come from this crate.
        let status = unsafe {
            if matches!(mode, 1 | 3 | 5) {
                z::deflateInit2_(
                    &raw mut *stream,
                    level,
                    8,
                    window,
                    memory,
                    strategy,
                    z::zlibVersion(),
                    STREAM_SIZE,
                )
            } else {
                z::inflateInit2_(&raw mut *stream, window, z::zlibVersion(), STREAM_SIZE)
            }
        };
        if status != 0 {
            return Err(error(&stream, status));
        }
        let mut codec = Self {
            stream,
            mode,
            dictionary,
            ended: false,
            gzip: (mode != 7).then_some(mode == 4),
        };
        if matches!(mode, 1 | 5 | 6) {
            codec.set_dictionary()?;
        }
        Ok(codec)
    }

    fn compressing(&self) -> bool {
        matches!(self.mode, 1 | 3 | 5)
    }

    fn set_dictionary(&mut self) -> Result<(), CodecError> {
        if self.dictionary.is_empty() {
            return Ok(());
        }
        let length = u32::try_from(self.dictionary.len())
            .map_err(|_| CodecError::new("Z_STREAM_ERROR", "Dictionary is too large"))?;
        // SAFETY: The initialized stream and dictionary slice remain valid for
        // this call; zlib copies the dictionary into its own state.
        let status = unsafe {
            if self.compressing() {
                z::deflateSetDictionary(&raw mut *self.stream, self.dictionary.as_ptr(), length)
            } else {
                z::inflateSetDictionary(&raw mut *self.stream, self.dictionary.as_ptr(), length)
            }
        };
        if status != 0 {
            return Err(error(&self.stream, status));
        }
        Ok(())
    }

    pub(super) fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: i32,
    ) -> Result<Step, CodecError> {
        if self.gzip.is_none() && !input.is_empty() {
            self.gzip = Some(input[0] == 0x1f);
        }
        if self.ended {
            if self.gzip != Some(true) || input.is_empty() || input[0] == 0 {
                return Ok(Step {
                    consumed: 0,
                    written: 0,
                    ended: true,
                });
            }
            self.reset()?;
        }
        let input_length = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let output_length = u32::try_from(output.len()).unwrap_or(u32::MAX);
        self.stream.next_in = input.as_ptr().cast_mut();
        self.stream.avail_in = input_length;
        self.stream.next_out = output.as_mut_ptr();
        self.stream.avail_out = output_length;
        // SAFETY: Both slices are live and disjoint throughout this call. Their
        // lengths bound zlib's accesses; pointers are cleared before returning.
        let mut status = unsafe {
            if self.compressing() {
                z::deflate(&raw mut *self.stream, flush)
            } else {
                z::inflate(&raw mut *self.stream, flush)
            }
        };
        if status == 2 && !self.dictionary.is_empty() {
            let dictionary_status = self.set_dictionary();
            if dictionary_status.is_ok() {
                // SAFETY: The same bounded input/output slices are still live.
                status = unsafe { z::inflate(&raw mut *self.stream, flush) };
            } else {
                status = -3;
            }
        }
        let consumed = (input_length - self.stream.avail_in) as usize;
        let written = (output_length - self.stream.avail_out) as usize;
        self.stream.next_in = ptr::null_mut();
        self.stream.next_out = ptr::null_mut();
        self.stream.avail_in = 0;
        self.stream.avail_out = 0;
        if !matches!(status, -5 | 0 | 1) {
            return Err(error(&self.stream, status));
        }
        self.ended = status == 1;
        if flush == 4 && !self.ended && written < output.len() {
            return Err(CodecError::new("Z_BUF_ERROR", "unexpected end of file"));
        }
        let next_member =
            self.gzip == Some(true) && input.get(consumed).is_some_and(|byte| *byte != 0);
        Ok(Step {
            consumed,
            written,
            ended: self.ended && !next_member,
        })
    }

    pub(super) fn reset(&mut self) -> Result<(), CodecError> {
        // SAFETY: This is an initialized stream; no borrowed buffers are live.
        let status = unsafe {
            if self.compressing() {
                z::deflateReset(&raw mut *self.stream)
            } else {
                z::inflateReset(&raw mut *self.stream)
            }
        };
        if status != 0 {
            return Err(error(&self.stream, status));
        }
        self.ended = false;
        if matches!(self.mode, 1 | 5 | 6) {
            self.set_dictionary()?;
        }
        Ok(())
    }

    pub(super) fn params(&mut self, level: i32, strategy: i32) -> Result<Vec<u8>, CodecError> {
        if !self.compressing() {
            return Ok(Vec::new());
        }
        let mut output = [0; 64];
        self.stream.next_out = output.as_mut_ptr();
        self.stream.avail_out = 64;
        // SAFETY: The caller flushes before changing parameters; no borrowed
        // buffers remain. zlib reports failure if further output is required.
        let status = unsafe { z::deflateParams(&raw mut *self.stream, level, strategy) };
        let written = 64 - self.stream.avail_out as usize;
        self.stream.next_out = ptr::null_mut();
        self.stream.avail_out = 0;
        if status != 0 {
            return Err(error(&self.stream, status));
        }
        Ok(output[..written].to_vec())
    }
}

impl Drop for Zlib {
    fn drop(&mut self) {
        // SAFETY: Initialization succeeded and this owner calls End exactly once.
        unsafe {
            if self.compressing() {
                z::deflateEnd(&raw mut *self.stream);
            } else {
                z::inflateEnd(&raw mut *self.stream);
            }
        }
    }
}

fn error(stream: &z::z_stream, status: i32) -> CodecError {
    let code = match status {
        2 => "Z_NEED_DICT",
        -2 => "Z_STREAM_ERROR",
        -3 => "Z_DATA_ERROR",
        -4 => "Z_MEM_ERROR",
        -5 => "Z_BUF_ERROR",
        _ => "Z_VERSION_ERROR",
    };
    let message = if stream.msg.is_null() {
        code.to_owned()
    } else {
        // SAFETY: zlib owns this NUL-terminated message for the stream's lifetime.
        unsafe { CStr::from_ptr(stream.msg) }
            .to_string_lossy()
            .into_owned()
    };
    CodecError::new(code, &message)
}
