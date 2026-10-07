//! Streaming content decoding of fetch response bodies.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use crate::globals::ContentDecoder;

const INPUT_CAPACITY: usize = 16 * 1024;

/// A response body decoded as it is read.
pub(crate) struct DecodedBody {
    body: Pin<Box<dyn AsyncRead>>,
    decoder: ContentDecoder,
    input: Vec<u8>,
    consumed: usize,
    ended: bool,
}

impl DecodedBody {
    pub(crate) fn new(body: Pin<Box<dyn AsyncRead>>, decoder: ContentDecoder) -> Self {
        Self {
            body,
            decoder,
            input: Vec::with_capacity(INPUT_CAPACITY),
            consumed: 0,
            ended: false,
        }
    }

    /// Decodes buffered input into `output`, reporting the bytes written.
    fn decode(&mut self, output: &mut [u8], finish: bool) -> io::Result<usize> {
        let step = self
            .decoder
            .step(&self.input[self.consumed..], output, finish)?;
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

impl AsyncRead for DecodedBody {
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
                let written = this.decode(buf.initialize_unfilled(), false)?;
                buf.advance(written);
                if written > 0 || this.ended {
                    return Poll::Ready(Ok(()));
                }
                continue;
            }
            if std::task::ready!(this.fill(cx))? == 0 {
                let written = this.decode(buf.initialize_unfilled(), true)?;
                buf.advance(written);
                return Poll::Ready(Ok(()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::AsyncReadExt;

    // "hello, hello, hello" compressed with `brotli -q 5`.
    const BROTLI: &[u8] = &[
        0x1f, 0x12, 0x00, 0x00, 0xa4, 0x40, 0x58, 0x6a, 0x90, 0x68, 0x2a, 0xf1, 0x9c, 0x2e,
    ];

    fn gzip(text: &str) -> io::Result<Vec<u8>> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text.as_bytes())?;
        encoder.finish()
    }

    /// `body` decoded from `encoding`, delivered one byte per read.
    async fn decode(encoding: &[u8], body: Vec<u8>) -> io::Result<Vec<u8>> {
        let decoder = ContentDecoder::new(encoding)?.ok_or(io::ErrorKind::Unsupported)?;
        let (mut writer, reader) = tokio::io::duplex(1);
        let producer = tokio::spawn(async move {
            for byte in body {
                tokio::io::AsyncWriteExt::write_all(&mut writer, &[byte]).await?;
            }
            Ok::<_, io::Error>(())
        });
        let mut output = Vec::new();
        let outcome = DecodedBody::new(Box::pin(reader), decoder)
            .read_to_end(&mut output)
            .await;
        // A body that fails to decode is no longer read, which fails its producer.
        let delivery = producer.await?;
        outcome?;
        delivery.map(|()| output)
    }

    fn kind<T>(result: io::Result<T>) -> Option<io::ErrorKind> {
        result.err().map(|error| error.kind())
    }

    #[tokio::test]
    async fn decodes_a_brotli_body_delivered_in_pieces() -> io::Result<()> {
        assert_eq!(
            decode(b"br", BROTLI.to_vec()).await?,
            b"hello, hello, hello"
        );
        Ok(())
    }

    #[tokio::test]
    async fn rejects_a_truncated_brotli_body() {
        let truncated = BROTLI[..BROTLI.len() - 3].to_vec();
        assert_eq!(
            kind(decode(b"br", truncated).await),
            Some(io::ErrorKind::InvalidData)
        );
    }

    #[tokio::test]
    async fn decodes_gzip_members_that_end_between_reads() -> io::Result<()> {
        let body = [gzip("hello, ")?, gzip("world")?].concat();
        assert_eq!(decode(b"gzip", body).await?, b"hello, world");
        Ok(())
    }

    #[tokio::test]
    async fn rejects_padding_after_a_gzip_member() -> io::Result<()> {
        let padded = [gzip("hello")?, vec![0, 0]].concat();
        assert_eq!(
            kind(decode(b"gzip", padded).await),
            Some(io::ErrorKind::InvalidData)
        );
        Ok(())
    }

    #[tokio::test]
    async fn rejects_truncated_and_empty_gzip_bodies() -> io::Result<()> {
        let body = gzip("hello")?;
        let truncated = body[..body.len() - 3].to_vec();
        assert_eq!(
            kind(decode(b"gzip", truncated).await),
            Some(io::ErrorKind::InvalidData)
        );
        assert_eq!(
            kind(decode(b"gzip", Vec::new()).await),
            Some(io::ErrorKind::InvalidData)
        );
        Ok(())
    }

    #[test]
    fn passes_other_encodings_through() -> io::Result<()> {
        for encoding in [&b"deflate"[..], b"zstd", b"identity", b"GZIP", b"x-gzip"] {
            assert!(ContentDecoder::new(encoding)?.is_none());
        }
        Ok(())
    }
}
