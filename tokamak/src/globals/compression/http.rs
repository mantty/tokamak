use std::collections::BTreeMap;
use std::io;

use super::node::{CodecError, Step};
use super::{brotli::Brotli, zlib::Zlib};

/// The HTTP content codings, over the same owned native codecs as node:zlib.
enum Codec {
    Gzip(Zlib),
    Brotli(Brotli),
}

impl Codec {
    fn step(&mut self, input: &[u8], output: &mut [u8], finish: bool) -> io::Result<Step> {
        match self {
            Self::Gzip(codec) => codec.step(input, output, if finish { 4 } else { 0 }),
            Self::Brotli(codec) => codec.step(input, output, if finish { 2 } else { 0 }),
        }
        .map_err(invalid_data)
    }
}

fn invalid_data(error: CodecError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.message)
}

/// Encodes a Worker response body's content coding.
pub(crate) struct ResponseEncoder {
    codec: Codec,
    started: bool,
}

impl ResponseEncoder {
    pub(crate) fn new(encoding: &str) -> io::Result<Option<Self>> {
        let codec = match encoding {
            "gzip" => Zlib::new(3, 15, -1, 8, 0, Vec::new()).map(Codec::Gzip),
            "br" => Brotli::new(true, &BTreeMap::from([(1, 5), (2, 19)])).map(Codec::Brotli),
            _ => return Ok(None),
        }
        .map_err(invalid_data)?;
        Ok(Some(Self {
            codec,
            started: false,
        }))
    }

    /// Consume input into one bounded output chunk. The boolean is true once
    /// this input (or the final flush) has been drained completely.
    pub(crate) fn step(
        &mut self,
        input: &[u8],
        finish: bool,
    ) -> io::Result<(usize, Vec<u8>, bool)> {
        self.started |= !input.is_empty();
        // Workerd does not emit a compressed frame for an untouched body.
        if !self.started {
            return Ok((0, Vec::new(), true));
        }
        let mut output = vec![0; 16384];
        let result = self.codec.step(input, &mut output, finish)?;
        let drained = result.ended
            || (!finish && result.consumed == input.len() && result.written < output.len());
        output.truncate(result.written);
        Ok((result.consumed, output, drained))
    }
}

/// Decodes a fetched response body's content coding.
pub(crate) struct ContentDecoder(Codec);

impl ContentDecoder {
    /// The decoder for `encoding`, or `None` when fetch passes the body through.
    pub(crate) fn new(encoding: &[u8]) -> io::Result<Option<Self>> {
        let codec = match encoding {
            b"gzip" => Zlib::new(4, 15, 0, 0, 0, Vec::new()).map(Codec::Gzip),
            b"br" => Brotli::new(false, &BTreeMap::new()).map(Codec::Brotli),
            _ => return Ok(None),
        }
        .map_err(invalid_data)?;
        Ok(Some(Self(codec)))
    }

    /// Decodes `input` into `output`; `finish` marks the end of the body. A
    /// gzip body is a series of members, so it ends only with the body.
    pub(crate) fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        finish: bool,
    ) -> io::Result<Step> {
        let step = self.0.step(input, output, finish)?;
        if !matches!(self.0, Codec::Gzip(_)) {
            return Ok(step);
        }
        if step.ended && step.consumed < input.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid gzip header",
            ));
        }
        Ok(Step {
            ended: step.ended && finish,
            ..step
        })
    }
}
