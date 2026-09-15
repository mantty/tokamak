use std::collections::BTreeMap;
use std::io;

use super::{brotli::Brotli, zlib::Zlib};

enum Codec {
    Gzip(Zlib),
    Brotli(Brotli),
}

/// HTTP content encoding uses the same owned native codecs as node:zlib.
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
        .map_err(|error| io::Error::other(error.message))?;
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
        let result = match &mut self.codec {
            Codec::Gzip(codec) => codec.step(input, &mut output, if finish { 4 } else { 0 }),
            Codec::Brotli(codec) => codec.step(input, &mut output, if finish { 2 } else { 0 }),
        }
        .map_err(|error| io::Error::other(error.message))?;
        let drained = result.ended
            || (!finish && result.consumed == input.len() && result.written < output.len());
        output.truncate(result.written);
        Ok((result.consumed, output, drained))
    }
}
