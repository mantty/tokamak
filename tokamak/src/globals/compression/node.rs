use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rquickjs::{Ctx, Exception, Function, Object, TypedArray};
use serde::Deserialize;

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
        match Exception::from_message(ctx.clone(), &self.message) {
            Ok(error) => {
                if let Err(error) = error.set("code", self.code) {
                    return error;
                }
                error.throw()
            }
            Err(error) => error,
        }
    }
}

pub(crate) struct Step {
    pub consumed: usize,
    pub written: usize,
    pub ended: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Options {
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
    fn new(mode: u8, options: &Options, dictionary: Vec<u8>) -> Result<Self, CodecError> {
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

// Arguments are owned conversions from JavaScript.
#[allow(clippy::needless_pass_by_value)]
pub(in crate::globals) fn create<'js>(
    ctx: Ctx<'js>,
    mode: u8,
    options: String,
    dictionary: TypedArray<'js, u8>,
) -> rquickjs::Result<Object<'js>> {
    let options: Options = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let dictionary = dictionary
        .as_bytes()
        .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression dictionary"))?
        .to_vec();
    let codec = Rc::new(RefCell::new(Some(
        Codec::new(mode, &options, dictionary.clone()).map_err(|error| error.throw(&ctx))?,
    )));
    let result = Object::new(ctx.clone())?;
    let writer = Rc::clone(&codec);
    result.set(
        "step",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, input: TypedArray<'js, u8>, flush: i32, capacity: usize| {
                let input = input
                    .as_bytes()
                    .ok_or_else(|| Exception::throw_type(&ctx, "Detached compression input"))?;
                let mut output = vec![0; capacity.clamp(64, 65536)];
                let mut state = writer.borrow_mut();
                let step = state
                    .as_mut()
                    .ok_or_else(|| Exception::throw_type(&ctx, "Compression stream is closed"))?
                    .step(input, &mut output, flush)
                    .map_err(|error| error.throw(&ctx))?;
                output.truncate(step.written);
                let result = Object::new(ctx.clone())?;
                result.set("consumed", step.consumed)?;
                result.set("ended", step.ended)?;
                result.set("output", TypedArray::new(ctx, output)?)?;
                Ok::<_, rquickjs::Error>(result)
            },
        )?,
    )?;
    let reset = Rc::clone(&codec);
    result.set(
        "reset",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
            let mut state = reset.borrow_mut();
            if let Some(Codec::Zlib(codec)) = state.as_mut() {
                codec.reset().map_err(|error| error.throw(&ctx))?;
            } else {
                *state = Some(
                    Codec::new(mode, &options, dictionary.clone())
                        .map_err(|error| error.throw(&ctx))?,
                );
            }
            Ok::<_, rquickjs::Error>(())
        })?,
    )?;
    let params = Rc::clone(&codec);
    result.set(
        "params",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, level: i32, strategy: i32| {
                let output = if let Some(Codec::Zlib(codec)) = params.borrow_mut().as_mut() {
                    codec
                        .params(level, strategy)
                        .map_err(|error| error.throw(&ctx))?
                } else {
                    Vec::new()
                };
                TypedArray::new(ctx, output)
            },
        )?,
    )?;
    result.set(
        "close",
        Function::new(ctx, move || {
            codec.borrow_mut().take();
        })?,
    )?;
    Ok(result)
}
