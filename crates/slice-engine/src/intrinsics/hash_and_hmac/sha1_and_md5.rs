//! Zero-Allocation Streaming FIPS 180-4 `SHA-1` and RFC 1321 `MD5` State Machines & `HMAC`.

use super::{BlockHasher, hmac_compute};

/// Zero-allocation FIPS 180-4 SHA-1 state machine (for WebCrypto `"SHA-1"` and WebSocket handshakes).
pub(crate) struct Sha1State {
    h: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Sha1State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            h: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(h: &mut [u32; 5], chunk: &[u8]) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = if i < 20 {
                ((b & c) | ((!b) & d), 0x5a827999u32)
            } else if i < 40 {
                (b ^ c ^ d, 0x6ed9eba1u32)
            } else if i < 60 {
                ((b & c) | (b & d) | (c & d), 0x8f1bbcdcu32)
            } else {
                (b ^ c ^ d, 0xca62c1d6u32)
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<64>();
        for chunk in chunks {
            Self::compress_block(&mut self.h, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize(mut self) -> [u8; 20] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..56].fill(0);
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);
        let mut out = [0u8; 20];
        for (idx, word) in self.h.iter().enumerate() {
            out[idx * 4..idx * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }
}

impl BlockHasher<64, 20> for Sha1State {
    #[inline(always)]
    fn init() -> Self {
        Self::new()
    }

    #[inline(always)]
    fn feed(&mut self, data: &[u8]) {
        self.update(data);
    }

    #[inline(always)]
    fn finish(self) -> [u8; 20] {
        self.finalize()
    }
}

#[inline]
pub(crate) fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut st = Sha1State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_sha1_bytes(key: &[u8], message: &[u8]) -> [u8; 20] {
    hmac_compute::<Sha1State, 64, 20>(key, message)
}

/// Zero-allocation RFC 1321 MD5 digest state machine (16-byte output) for `node:crypto`
/// `createHash("md5")` and `createHmac("md5", ...)` compatibility.
pub(crate) struct Md5State {
    s: [u32; 4],
    buf: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Md5State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            s: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(s: &mut [u32; 4], chunk: &[u8]) {
        const S_SHIFT: [u32; 64] = [
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20,
            5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23,
            6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
        ];
        const K_TAB: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
        ];
        let mut m = [0u32; 16];
        for (i, item) in m.iter_mut().enumerate() {
            let off = i * 4;
            *item =
                u32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
        }
        let (mut a, mut b, mut c, mut d) = (s[0], s[1], s[2], s[3]);
        for i in 0..64 {
            let (f, g) = if i < 16 {
                ((b & c) | ((!b) & d), i)
            } else if i < 32 {
                ((d & b) | ((!d) & c), (5 * i + 1) % 16)
            } else if i < 48 {
                (b ^ c ^ d, (3 * i + 5) % 16)
            } else {
                (c ^ (b | (!d)), (7 * i) % 16)
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K_TAB[i])
                    .wrapping_add(m[g])
                    .rotate_left(S_SHIFT[i]),
            );
            a = temp;
        }
        s[0] = s[0].wrapping_add(a);
        s[1] = s[1].wrapping_add(b);
        s[2] = s[2].wrapping_add(c);
        s[3] = s[3].wrapping_add(d);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buf_len > 0 {
            let need = 64 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..64].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.s, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<64>();
        for chunk in chunks {
            Self::compress_block(&mut self.s, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize(mut self) -> [u8; 16] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 56 {
            self.buf[self.buf_len..64].fill(0);
            Self::compress_block(&mut self.s, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..56].fill(0);
        }
        self.buf[56..64].copy_from_slice(&bit_len.to_le_bytes());
        Self::compress_block(&mut self.s, &self.buf);
        let mut out = [0u8; 16];
        for (i, word) in self.s.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

impl BlockHasher<64, 16> for Md5State {
    #[inline(always)]
    fn init() -> Self {
        Self::new()
    }

    #[inline(always)]
    fn feed(&mut self, data: &[u8]) {
        self.update(data);
    }

    #[inline(always)]
    fn finish(self) -> [u8; 16] {
        self.finalize()
    }
}

#[inline]
pub(crate) fn md5_bytes(data: &[u8]) -> [u8; 16] {
    let mut st = Md5State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_md5_bytes(key: &[u8], message: &[u8]) -> [u8; 16] {
    hmac_compute::<Md5State, 64, 16>(key, message)
}
