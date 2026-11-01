use std::collections::BTreeMap;
use tokamak_macros::{JsType, host_function};

use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, Exception, IntoJs, JsLifetime, TypedArray};
use serde::Deserialize;

use crate::typed::Serde;

use super::{brotli::Brotli, zlib::Zlib, zstd::Zstd};

#[derive(Debug)]
pub(crate) struct CodecError {
    pub code: String,
    pub message: String,
}

impl CodecError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    fn throw(self, ctx: &Ctx<'_>) -> rquickjs::Error {
        crate::errors::new_error(ctx, &self.message)
            .and_then(|error| error.set("code", self.code).map(|()| error.throw()))
            .unwrap_or_else(|error| error)
    }
}

pub(crate) struct Step {
    pub consumed: usize,
    pub written: usize,
    pub ended: bool,
}

/// The settings of a node:zlib codec.
#[derive(Deserialize, JsType)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodecOptions {
    pub window_bits: i32,
    pub level: i32,
    pub mem_level: i32,
    pub strategy: i32,
    pub params: BTreeMap<u32, i32>,
    pub pledged_src_size: Option<u64>,
}

enum Codec {
    Zlib(Zlib),
    Brotli(Brotli),
    Zstd(Zstd),
}

impl Codec {
    fn new(mode: u8, options: &CodecOptions, dictionary: Vec<u8>) -> Result<Self, CodecError> {
        match mode {
            1..=7 => Zlib::new(
                mode,
                options.window_bits,
                options.level,
                options.mem_level,
                options.strategy,
                dictionary,
            )
            .map(Self::Zlib),
            8 | 9 => Brotli::new(mode == 9, &options.params).map(Self::Brotli),
            10 | 11 => Zstd::new(mode == 10, options)
                .map(Self::Zstd)
                .map_err(|error| CodecError::new("ERR_ZLIB_INITIALIZATION_FAILED", &error.message)),
            _ => Err(CodecError::new(
                "Z_STREAM_ERROR",
                "Invalid compression mode",
            )),
        }
    }

    fn step(&mut self, input: &[u8], output: &mut [u8], flush: i32) -> Result<Step, CodecError> {
        match self {
            Self::Zlib(codec) => codec.step(input, output, flush),
            Self::Brotli(codec) => codec.step(input, output, flush),
            Self::Zstd(codec) => codec.step(input, output, flush),
        }
    }
}

/// The codec of a node:zlib stream of `mode`, from 1 to 11 as the zlib
/// constants number them.
// Arguments are owned conversions from JavaScript.
#[allow(clippy::needless_pass_by_value)]
#[host_function]
pub(in crate::globals) fn create<'js>(
    ctx: Ctx<'js>,
    mode: u8,
    options: Serde<CodecOptions>,
    dictionary: TypedArray<'js, u8>,
) -> rquickjs::Result<Class<'js, NodeCodec>> {
    let Serde(options) = options;
    let dictionary = dictionary
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression dictionary"))?
        .to_vec();
    let codec =
        Codec::new(mode, &options, dictionary.clone()).map_err(|error| error.throw(&ctx))?;
    Class::instance(
        ctx,
        NodeCodec {
            codec: Some(codec),
            mode,
            options,
            dictionary,
        },
    )
}

/// A node:zlib stream's codec, with the settings that recreate it.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct NodeCodec {
    /// None once closed.
    #[qjs(skip_trace)]
    codec: Option<Codec>,
    #[qjs(skip_trace)]
    mode: u8,
    #[qjs(skip_trace)]
    options: CodecOptions,
    #[qjs(skip_trace)]
    dictionary: Vec<u8>,
}

/// The input a codec step consumed, whether the stream ended, and the output.
#[derive(IntoJs, JsType)]
pub(crate) struct CodecStep<'js> {
    consumed: usize,
    ended: bool,
    output: TypedArray<'js, u8>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl NodeCodec {
    /// Passes `input` through the codec with the zlib flush value `flush`,
    /// writing at most `capacity` bytes, clamped to 64 through 65536.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn step<'js>(
        &mut self,
        ctx: Ctx<'js>,
        input: TypedArray<'js, u8>,
        flush: i32,
        capacity: usize,
    ) -> rquickjs::Result<CodecStep<'js>> {
        let input = input
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression input"))?;
        let mut output = vec![0; capacity.clamp(64, 65536)];
        let step = self
            .codec
            .as_mut()
            .ok_or_else(|| Exception::throw_type(&ctx, "Compression stream is closed"))?
            .step(input, &mut output, flush)
            .map_err(|error| error.throw(&ctx))?;
        output.truncate(step.written);
        Ok(CodecStep {
            consumed: step.consumed,
            ended: step.ended,
            output: TypedArray::new(ctx, output)?,
        })
    }

    /// Returns the codec to its initial state; a zlib codec resets in place,
    /// and others, or a closed codec, are recreated.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn reset(&mut self, ctx: Ctx<'_>) -> rquickjs::Result<()> {
        if let Some(Codec::Zlib(codec)) = self.codec.as_mut() {
            return codec.reset().map_err(|error| error.throw(&ctx));
        }
        let codec = Codec::new(self.mode, &self.options, self.dictionary.clone())
            .map_err(|error| error.throw(&ctx))?;
        self.codec = Some(codec);
        Ok(())
    }

    /// Changes a zlib codec's level and strategy, returning the output that
    /// flushes; other codecs return no output.
    pub fn params<'js>(
        &mut self,
        ctx: Ctx<'js>,
        level: i32,
        strategy: i32,
    ) -> rquickjs::Result<TypedArray<'js, u8>> {
        let output = match self.codec.as_mut() {
            Some(Codec::Zlib(codec)) => codec
                .params(level, strategy)
                .map_err(|error| error.throw(&ctx))?,
            _ => Vec::new(),
        };
        TypedArray::new(ctx, output)
    }

    /// Releases the codec.
    pub fn close(&mut self) {
        self.codec = None;
    }
}
