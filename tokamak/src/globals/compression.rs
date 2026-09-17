pub(crate) mod brotli;
pub(super) mod http;
pub(super) mod node;
mod zlib;
mod zstd;

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};
use rquickjs::{Ctx, Exception, Function, TypedArray, function::MutFn};

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
        let status = match self {
            Self::Compress(codec) => codec
                .compress(
                    input,
                    output,
                    if finish {
                        FlushCompress::Finish
                    } else {
                        FlushCompress::None
                    },
                )
                .map_err(|error| error.to_string())?,
            Self::Decompress(codec) => codec
                .decompress(
                    input,
                    output,
                    if finish {
                        FlushDecompress::Finish
                    } else {
                        FlushDecompress::None
                    },
                )
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
pub(super) fn create<'js>(
    ctx: Ctx<'js>,
    format: String,
    decode: bool,
) -> rquickjs::Result<Function<'js>> {
    let mut codec = match (format.as_str(), decode) {
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
    let mut ended = false;
    Function::new(
        ctx,
        MutFn::new(
            move |ctx: Ctx<'js>, input: TypedArray<'js, u8>, finish: bool, emit: Function<'js>| {
                let input = input
                    .as_bytes()
                    .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression input"))?
                    .to_vec();
                let mut remaining = input.as_slice();
                if ended && !remaining.is_empty() {
                    return Err(Exception::throw_type(
                        &ctx,
                        "Trailing bytes after end of compressed data",
                    ));
                }
                let mut output = [0; 16384];
                while !ended {
                    let (consumed, written, status) = codec
                        .step(remaining, &mut output, finish)
                        .map_err(|error| Exception::throw_type(&ctx, &error))?;
                    remaining = &remaining[consumed..];
                    if written > 0 {
                        emit.call::<_, ()>((TypedArray::new(
                            ctx.clone(),
                            output[..written].to_vec(),
                        )?,))?;
                    }
                    ended = status == Status::StreamEnd;
                    if ended && !remaining.is_empty() {
                        return Err(Exception::throw_type(
                            &ctx,
                            "Trailing bytes after end of compressed data",
                        ));
                    }
                    if consumed == 0 && written == 0 {
                        if finish && !ended {
                            return Err(Exception::throw_type(
                                &ctx,
                                "Called close() on a decompression stream with incomplete data",
                            ));
                        }
                        break;
                    }
                }
                Ok(())
            },
        ),
    )
}
