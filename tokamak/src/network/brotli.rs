//! Streaming Brotli decoding of fetch response bodies over the linked C codec.

use std::collections::BTreeMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use crate::globals::compression::brotli::Brotli;

const INPUT_CAPACITY: usize = 16 * 1024;
const PROCESS: i32 = 0;
const FINISH: i32 = 2;

pub(crate) struct Decoder {
    body: Pin<Box<dyn AsyncRead + Send>>,
    codec: Result<Brotli, io::Error>,
    input: Vec<u8>,
    consumed: usize,
    ended: bool,
}

impl Decoder {
    pub(crate) fn new(body: Pin<Box<dyn AsyncRead + Send>>) -> Self {
        Self {
            body,
            codec: Brotli::new(false, &BTreeMap::new())
                .map_err(|error| io::Error::other(error.message)),
            input: Vec::with_capacity(INPUT_CAPACITY),
            consumed: 0,
            ended: false,
        }
    }

    /// Decodes buffered input into `output`, reporting the bytes written.
    fn decode(&mut self, output: &mut [u8], flush: i32) -> io::Result<usize> {
        let codec = self
            .codec
            .as_mut()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let step = codec
            .step(&self.input[self.consumed..], output, flush)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.message))?;
        self.consumed += step.consumed;
        self.ended = step.ended;
        Ok(step.written)
    }

    fn fill(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        self.input.clear();
        self.input.resize(INPUT_CAPACITY, 0);
        self.consumed = 0;
        let mut buffer = ReadBuf::new(&mut self.input);
        let result = self.body.as_mut().poll_read(cx, &mut buffer);
        let length = buffer.filled().len();
        self.input.truncate(length);
        result.map_ok(|()| length)
    }
}

impl AsyncRead for Decoder {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.ended || buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }
            if this.consumed < this.input.len() {
                let written = this.decode(buf.initialize_unfilled(), PROCESS)?;
                buf.advance(written);
                if written > 0 || this.ended {
                    return Poll::Ready(Ok(()));
                }
                continue;
            }
            match this.fill(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(0)) => {
                    let written = this.decode(buf.initialize_unfilled(), FINISH)?;
                    buf.advance(written);
                    return Poll::Ready(Ok(()));
                }
                Poll::Ready(Ok(_)) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    // "hello, hello, hello" compressed with `brotli -q 5`.
    const COMPRESSED: &[u8] = &[
        0x1f, 0x12, 0x00, 0x00, 0xa4, 0x40, 0x58, 0x6a, 0x90, 0x68, 0x2a, 0xf1, 0x9c, 0x2e,
    ];

    #[tokio::test]
    async fn decodes_a_body_delivered_in_pieces() -> io::Result<()> {
        let (mut writer, reader) = tokio::io::duplex(4);
        let producer = tokio::spawn(async move {
            for byte in COMPRESSED {
                tokio::io::AsyncWriteExt::write_all(&mut writer, &[*byte]).await?;
            }
            Ok::<_, io::Error>(())
        });
        let mut output = String::new();
        Decoder::new(Box::pin(reader))
            .read_to_string(&mut output)
            .await?;
        producer.await??;
        assert_eq!(output, "hello, hello, hello");
        Ok(())
    }

    #[tokio::test]
    async fn rejects_a_truncated_body() {
        let mut output = Vec::new();
        let result = Decoder::new(Box::pin(&COMPRESSED[..COMPRESSED.len() - 3]))
            .read_to_end(&mut output)
            .await;
        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(io::ErrorKind::InvalidData)
        );
    }
}
