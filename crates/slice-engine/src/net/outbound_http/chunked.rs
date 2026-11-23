//! Incremental O(N) HTTP/1.1 `Transfer-Encoding: chunked` Stream Decoder.

use super::MAX_OUTBOUND_HEADER_BYTES;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChunkDecodeState {
    ReadingSizeLine,
    ReadingData { remaining: usize },
    ReadingDataCrlf { got_cr: bool },
    ReadingTrailers,
    Done,
}

/// Incremental O(N) HTTP/1.1 `Transfer-Encoding: chunked` stream decoder with checked arithmetic,
/// strict hex length validation (`1..=16` hex digits), and bounded decoded payload size.
pub struct ChunkedBodyDecoder {
    state: ChunkDecodeState,
    decoded: Vec<u8>,
    max_decoded_bytes: usize,
}

impl ChunkedBodyDecoder {
    #[inline]
    pub fn new(max_decoded_bytes: usize) -> Self {
        Self {
            state: ChunkDecodeState::ReadingSizeLine,
            decoded: Vec::new(),
            max_decoded_bytes,
        }
    }

    /// Feeds newly available raw bytes into the chunked state machine.
    /// Returns `Ok((bytes_consumed_from_buf, is_done))`.
    pub fn feed(&mut self, buf: &[u8]) -> Result<(usize, bool), std::io::Error> {
        let mut pos = 0usize;
        while pos < buf.len() {
            match self.state {
                ChunkDecodeState::ReadingSizeLine => {
                    let Some(rel_end) = memchr::memmem::find(&buf[pos..], b"\r\n") else {
                        if buf.len() - pos > 1024 {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "chunk header line exceeds 1024 bytes without CRLF",
                            ));
                        }
                        return Ok((pos, false));
                    };
                    let size_line =
                        std::str::from_utf8(&buf[pos..pos + rel_end]).map_err(|_| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "invalid UTF-8 in chunk size line",
                            )
                        })?;
                    let size_hex = size_line.split(';').next().unwrap_or("").trim();
                    if size_hex.is_empty()
                        || size_hex.len() > 16
                        || !size_hex.bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "malformed hexadecimal chunk size",
                        ));
                    }
                    let chunk_len = usize::from_str_radix(size_hex, 16).map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, "chunk size overflow")
                    })?;
                    pos = pos
                        .checked_add(rel_end)
                        .and_then(|v| v.checked_add(2))
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "chunk offset overflow",
                            )
                        })?;

                    if chunk_len == 0 {
                        self.state = ChunkDecodeState::ReadingTrailers;
                    } else {
                        let new_decoded_len =
                            self.decoded.len().checked_add(chunk_len).ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "decoded chunked total overflow",
                                )
                            })?;
                        if new_decoded_len > self.max_decoded_bytes {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::OutOfMemory,
                                "chunked HTTP body exceeded maximum memory budget",
                            ));
                        }
                        self.decoded.reserve(chunk_len.min(65536));
                        self.state = ChunkDecodeState::ReadingData {
                            remaining: chunk_len,
                        };
                    }
                }
                ChunkDecodeState::ReadingData { remaining } => {
                    let avail = buf.len() - pos;
                    let take = avail.min(remaining);
                    self.decoded.extend_from_slice(&buf[pos..pos + take]);
                    pos += take;
                    let left = remaining - take;
                    if left == 0 {
                        self.state = ChunkDecodeState::ReadingDataCrlf { got_cr: false };
                    } else {
                        self.state = ChunkDecodeState::ReadingData { remaining: left };
                    }
                }
                ChunkDecodeState::ReadingDataCrlf { got_cr } => {
                    if !got_cr {
                        if buf[pos] != b'\r' {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "missing CRLF after chunk payload",
                            ));
                        }
                        pos += 1;
                        if pos < buf.len() {
                            if buf[pos] != b'\n' {
                                return Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "missing CRLF after chunk payload",
                                ));
                            }
                            pos += 1;
                            self.state = ChunkDecodeState::ReadingSizeLine;
                        } else {
                            self.state = ChunkDecodeState::ReadingDataCrlf { got_cr: true };
                        }
                    } else {
                        if buf[pos] != b'\n' {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "missing CRLF after chunk payload",
                            ));
                        }
                        pos += 1;
                        self.state = ChunkDecodeState::ReadingSizeLine;
                    }
                }
                ChunkDecodeState::ReadingTrailers => {
                    if buf.len() - pos < 2 {
                        return Ok((pos, false));
                    }
                    if &buf[pos..pos + 2] == b"\r\n" {
                        pos += 2;
                        self.state = ChunkDecodeState::Done;
                        return Ok((pos, true));
                    }
                    if let Some(trailer_end) = memchr::memmem::find(&buf[pos..], b"\r\n\r\n") {
                        pos += trailer_end + 4;
                        self.state = ChunkDecodeState::Done;
                        return Ok((pos, true));
                    }
                    if buf.len() - pos > MAX_OUTBOUND_HEADER_BYTES {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "chunked trailer section exceeds maximum header budget",
                        ));
                    }
                    return Ok((pos, false));
                }
                ChunkDecodeState::Done => {
                    return Ok((pos, true));
                }
            }
        }
        Ok((pos, self.state == ChunkDecodeState::Done))
    }

    #[inline]
    pub fn into_vec(self) -> Vec<u8> {
        self.decoded
    }
}

/// Strictly parses an HTTP/1.1 `Transfer-Encoding: chunked` body with checked arithmetic (`N-Sec-5`),
/// strict hex length validation (`1..=16` hex digits), and bounded decoded payload size.
pub fn parse_chunked_body(
    raw: &[u8],
    max_decoded_bytes: usize,
) -> Result<Option<(Vec<u8>, usize)>, std::io::Error> {
    let mut decoder = ChunkedBodyDecoder::new(max_decoded_bytes);
    let (consumed, done) = decoder.feed(raw)?;
    if done {
        Ok(Some((decoder.into_vec(), consumed)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_body_decoder_handles_arbitrary_split_boundaries_and_trailers() {
        let payload_a = b"Hello, streaming ";
        let payload_b = b"HTTP/1.1 chunked decoder with ";
        let payload_c = vec![b'X'; 1024];
        let mut wire = Vec::new();
        wire.extend_from_slice(format!("{:x};ext=val\r\n", payload_a.len()).as_bytes());
        wire.extend_from_slice(payload_a);
        wire.extend_from_slice(b"\r\n");
        wire.extend_from_slice(format!("{:X}\r\n", payload_b.len()).as_bytes());
        wire.extend_from_slice(payload_b);
        wire.extend_from_slice(b"\r\n");
        wire.extend_from_slice(format!("{:x}\r\n", payload_c.len()).as_bytes());
        wire.extend_from_slice(&payload_c);
        wire.extend_from_slice(
            b"\r\n0;final=1\r\nX-Trailer-One: 1\r\nX-Trailer-Two: 2\r\n\r\nEXTRA",
        );

        let mut expected = Vec::new();
        expected.extend_from_slice(payload_a);
        expected.extend_from_slice(payload_b);
        expected.extend_from_slice(&payload_c);

        for step in [1usize, 2, 7, 13, 64, wire.len()] {
            let mut dec = ChunkedBodyDecoder::new(64 * 1024);
            let mut cursor = 0usize;
            let mut done = false;
            while cursor < wire.len() && !done {
                let end = (cursor + step).min(wire.len());
                let (consumed, finished) = dec.feed(&wire[cursor..end]).expect("valid chunk feed");
                cursor += consumed;
                done = finished;
                if !done && consumed == 0 {
                    let mut next_end = (end + step).min(wire.len());
                    while !done {
                        let (c2, f2) = dec
                            .feed(&wire[cursor..next_end])
                            .expect("valid expanded chunk feed");
                        cursor += c2;
                        done = f2;
                        if done || c2 > 0 || next_end == wire.len() {
                            break;
                        }
                        next_end = (next_end + step).min(wire.len());
                    }
                }
            }
            assert!(done, "decoder should finish for step={}", step);
            assert_eq!(&wire[cursor..], b"EXTRA");
            assert_eq!(dec.into_vec(), expected);
        }
    }

    #[test]
    fn chunked_body_decoder_enforces_size_limit_and_crlf_framing() {
        let oversized = b"10\r\n0123456789abcdef\r\n0\r\n\r\n";
        let err = parse_chunked_body(oversized, 8).expect_err("should reject over budget");
        assert_eq!(err.kind(), std::io::ErrorKind::OutOfMemory);

        let bad_crlf = b"4\r\nabcdXX0\r\n\r\n";
        let err_framing = parse_chunked_body(bad_crlf, 1024).expect_err("should reject bad CRLF");
        assert_eq!(err_framing.kind(), std::io::ErrorKind::InvalidData);
    }
}
