use zstd::zstd_safe::{self as z, CParameter as C, DParameter, InBuffer, OutBuffer};

use super::node::{CodecError, Options, Step};

pub(super) enum Zstd {
    Encoder(z::CCtx<'static>),
    Decoder {
        context: z::DCtx<'static>,
        frame_ended: bool,
    },
}

fn error(code: usize) -> CodecError {
    CodecError::new("ERR_ZSTD_COMPRESSION_FAILED", z::get_error_name(code))
}

impl Zstd {
    pub(super) fn new(encode: bool, options: &Options) -> Result<Self, CodecError> {
        if !encode {
            let mut decoder = z::DCtx::create();
            for (&key, &value) in &options.params {
                if key != 100 {
                    return Err(CodecError::new(
                        "ERR_ZSTD_INVALID_PARAM",
                        "Invalid Zstd decoder parameter",
                    ));
                }
                decoder
                    .set_parameter(DParameter::WindowLogMax(value.cast_unsigned()))
                    .map_err(error)?;
            }
            return Ok(Self::Decoder {
                context: decoder,
                frame_ended: false,
            });
        }
        let mut encoder = z::CCtx::create();
        for (&key, &value) in &options.params {
            encoder
                .set_parameter(parameter(key, value)?)
                .map_err(error)?;
        }
        encoder
            .set_pledged_src_size(options.pledged_src_size)
            .map_err(error)?;
        Ok(Self::Encoder(encoder))
    }

    pub(super) fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: i32,
    ) -> Result<Step, CodecError> {
        let mut input = InBuffer::around(input);
        let mut output = OutBuffer::around(output);
        let remaining = match self {
            Self::Encoder(encoder) => {
                let operation = match flush {
                    0 => z::zstd_sys::ZSTD_EndDirective::ZSTD_e_continue,
                    1 => z::zstd_sys::ZSTD_EndDirective::ZSTD_e_flush,
                    2 => z::zstd_sys::ZSTD_EndDirective::ZSTD_e_end,
                    _ => return Err(CodecError::new("Z_STREAM_ERROR", "Invalid Zstd operation")),
                };
                encoder
                    .compress_stream2(&mut output, &mut input, operation)
                    .map_err(error)?
            }
            Self::Decoder {
                context,
                frame_ended,
            } => {
                if *frame_ended && input.src.is_empty() {
                    return Ok(Step {
                        consumed: 0,
                        written: 0,
                        ended: true,
                    });
                }
                let remaining =
                    context
                        .decompress_stream(&mut output, &mut input)
                        .map_err(|code| {
                            CodecError::new(
                                "ERR_ZSTD_DECOMPRESSION_FAILED",
                                z::get_error_name(code),
                            )
                        })?;
                *frame_ended = remaining == 0;
                if flush == 2 && remaining != 0 && output.pos() < output.capacity() {
                    return Err(CodecError::new(
                        "ERR_ZSTD_DECOMPRESSION_FAILED",
                        "unexpected end of file",
                    ));
                }
                remaining
            }
        };
        Ok(Step {
            consumed: input.pos(),
            written: output.pos(),
            ended: remaining == 0 && flush == 2 && input.pos() == input.src.len(),
        })
    }
}

fn parameter(key: u32, value: i32) -> Result<C, CodecError> {
    let unsigned = value.cast_unsigned();
    Ok(match key {
        100 => C::CompressionLevel(value),
        101 => C::WindowLog(unsigned),
        102 => C::HashLog(unsigned),
        103 => C::ChainLog(unsigned),
        104 => C::SearchLog(unsigned),
        105 => C::MinMatch(unsigned),
        106 => C::TargetLength(unsigned),
        107 => C::Strategy(match value {
            1 => z::Strategy::ZSTD_fast,
            2 => z::Strategy::ZSTD_dfast,
            3 => z::Strategy::ZSTD_greedy,
            4 => z::Strategy::ZSTD_lazy,
            5 => z::Strategy::ZSTD_lazy2,
            6 => z::Strategy::ZSTD_btlazy2,
            7 => z::Strategy::ZSTD_btopt,
            8 => z::Strategy::ZSTD_btultra,
            9 => z::Strategy::ZSTD_btultra2,
            _ => {
                return Err(CodecError::new(
                    "ERR_ZSTD_COMPRESSION_FAILED",
                    "Invalid Zstd strategy",
                ));
            }
        }),
        160 => C::EnableLongDistanceMatching(value != 0),
        161 => C::LdmHashLog(unsigned),
        162 => C::LdmMinMatch(unsigned),
        163 => C::LdmBucketSizeLog(unsigned),
        164 => C::LdmHashRateLog(unsigned),
        200 => C::ContentSizeFlag(value != 0),
        201 => C::ChecksumFlag(value != 0),
        202 => C::DictIdFlag(value != 0),
        400 => C::NbWorkers(unsigned),
        401 => C::JobSize(unsigned),
        402 => C::OverlapSizeLog(unsigned),
        _ => {
            return Err(CodecError::new(
                "ERR_ZSTD_INVALID_PARAM",
                "Invalid Zstd parameter",
            ));
        }
    })
}
