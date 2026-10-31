mod brotli;
pub(super) mod http;
pub(super) mod node;
mod zlib;
mod zstd;

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, Exception, JsLifetime, TypedArray};
use tokamak_macros::host_function;

use crate::typed::JsFunction;

enum Codec {
    Compress(Compress),
    Decompress(Decompress),
}

impl Codec {
    // Codec counters advance by at most the supplied usize-sized slices.
    #[allow(clippy::cast_possible_truncation)]
    fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        finish: bool,
    ) -> Result<(usize, usize, Status), String> {
        let (before_in, before_out) = self.totals();
        let (compress_flush, decompress_flush) = if finish {
            (FlushCompress::Finish, FlushDecompress::Finish)
        } else {
            (FlushCompress::None, FlushDecompress::None)
        };
        let status = match self {
            Self::Compress(codec) => codec
                .compress(input, output, compress_flush)
                .map_err(|error| error.to_string())?,
            Self::Decompress(codec) => codec
                .decompress(input, output, decompress_flush)
                .map_err(|error| error.to_string())?,
        };
        let (after_in, after_out) = self.totals();
        Ok((
            (after_in - before_in) as usize,
            (after_out - before_out) as usize,
            status,
        ))
    }

    fn totals(&self) -> (u64, u64) {
        match self {
            Self::Compress(codec) => (codec.total_in(), codec.total_out()),
            Self::Decompress(codec) => (codec.total_in(), codec.total_out()),
        }
    }
}

// rquickjs converts JavaScript string arguments to owned Rust strings.
#[allow(clippy::needless_pass_by_value)]
#[host_function]
pub(super) fn create(
    ctx: Ctx<'_>,
    format: String,
    decode: bool,
) -> rquickjs::Result<Class<'_, CompressionCodec>> {
    let codec = match (format.as_str(), decode) {
        ("gzip", false) => Codec::Compress(Compress::new_gzip(Compression::default(), 15)),
        ("gzip", true) => Codec::Decompress(Decompress::new_gzip(15)),
        ("deflate", false) => Codec::Compress(Compress::new(Compression::default(), true)),
        ("deflate", true) => Codec::Decompress(Decompress::new(true)),
        ("deflate-raw", false) => Codec::Compress(Compress::new(Compression::default(), false)),
        ("deflate-raw", true) => Codec::Decompress(Decompress::new(false)),
        _ => {
            return Err(Exception::throw_type(
                &ctx,
                "The compression format must be either 'deflate', 'deflate-raw' or 'gzip'.",
            ));
        }
    };
    Class::instance(
        ctx,
        CompressionCodec {
            codec,
            ended: false,
        },
    )
}

/// A `CompressionStream`'s or `DecompressionStream`'s codec.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct CompressionCodec {
    #[qjs(skip_trace)]
    codec: Codec,
    ended: bool,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl CompressionCodec {
    /// Passes `input` through the codec, finishing the stream when `finish`
    /// is set, and calls `emit` with each block of output.
    pub fn write<'js>(
        &mut self,
        ctx: Ctx<'js>,
        input: TypedArray<'js, u8>,
        finish: bool,
        emit: JsFunction<'js, (TypedArray<'js, u8>,)>,
    ) -> rquickjs::Result<()> {
        let input = input
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression input"))?
            .to_vec();
        let mut remaining = input.as_slice();
        let mut output = [0; 16384];
        let mut stalled = false;
        while !self.ended && !stalled {
            let (consumed, written, status) = self
                .codec
                .step(remaining, &mut output, finish)
                .map_err(|error| Exception::throw_type(&ctx, &error))?;
            remaining = &remaining[consumed..];
            if written > 0 {
                emit.call((TypedArray::new(ctx.clone(), output[..written].to_vec())?,))?;
            }
            self.ended = status == Status::StreamEnd;
            stalled = !self.ended && consumed == 0 && written == 0;
        }
        if stalled && finish {
            return Err(Exception::throw_type(
                &ctx,
                "Called close() on a decompression stream with incomplete data",
            ));
        }
        if self.ended && !remaining.is_empty() {
            return Err(Exception::throw_type(
                &ctx,
                "Trailing bytes after end of compressed data",
            ));
        }
        Ok(())
    }
}
