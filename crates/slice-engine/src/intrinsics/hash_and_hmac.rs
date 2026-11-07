//! Zero-Allocation Streaming Hash & HMAC State Machines (`SHA-NI` + FIPS 180-4 SHA-256,
//! SHA-1, SHA-384, SHA-512, RFC 1321 MD5, and Constant-Time Slice Equality).

use super::encoding::digest_to_hex;

/// Phase 9A Zero-Allocation Streaming FIPS-180-4 SHA-256 State Machine.
/// Processes 64-byte chunks directly from borrowed `&[u8]` slices in place and buffers
/// only the final `<= 64` byte partial block on the stack (0 heap allocations).
pub(crate) struct Sha256State {
    pub(crate) h: [u32; 8],
    pub(crate) buf: [u8; 64],
    pub(crate) buf_len: usize,
    pub(crate) total_len: u64,
}

impl Sha256State {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    pub(crate) fn compress_block(h: &mut [u32; 8], chunk: &[u8]) {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            static SHA_NI_STATUS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let status = SHA_NI_STATUS.load(std::sync::atomic::Ordering::Relaxed);
            let use_sha_ni = if status == 1 {
                true
            } else if status == 2 {
                false
            } else {
                let detected = std::arch::is_x86_feature_detected!("sha")
                    && std::arch::is_x86_feature_detected!("sse4.1")
                    && std::arch::is_x86_feature_detected!("ssse3");
                SHA_NI_STATUS.store(
                    if detected { 1 } else { 2 },
                    std::sync::atomic::Ordering::Relaxed,
                );
                detected
            };
            if use_sha_ni {
                unsafe {
                    Self::compress_block_sha_ni(h, chunk);
                }
                return;
            }
        }
        Self::compress_block_soft(h, chunk);
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "sha,sse4.1,ssse3")]
    pub(crate) unsafe fn compress_block_sha_ni(h: &mut [u32; 8], chunk: &[u8]) {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::*;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::*;

        let shuf_mask = _mm_set_epi64x(0x0c0d0e0f08090a0b, 0x0405060700010203);

        let state0_in = _mm_loadu_si128(h.as_ptr() as *const __m128i);
        let state1_in = _mm_loadu_si128(h.as_ptr().add(4) as *const __m128i);

        let tmp = _mm_shuffle_epi32(state0_in, 0xB1);
        let state1_shuf = _mm_shuffle_epi32(state1_in, 0x1B);
        let mut state0 = _mm_alignr_epi8(tmp, state1_shuf, 8);
        let mut state1 = _mm_blend_epi16(state1_shuf, tmp, 0xF0);

        let abef_save = state0;
        let cdgh_save = state1;

        let ptr = chunk.as_ptr() as *const __m128i;
        let mut msg0 = _mm_shuffle_epi8(_mm_loadu_si128(ptr), shuf_mask);
        let mut msg1 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(1)), shuf_mask);
        let mut msg2 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(2)), shuf_mask);
        let mut msg3 = _mm_shuffle_epi8(_mm_loadu_si128(ptr.add(3)), shuf_mask);

        macro_rules! sha256_4rounds {
            ($msg:expr, $k3:expr, $k2:expr, $k1:expr, $k0:expr) => {{
                let msg_k = _mm_add_epi32(
                    $msg,
                    _mm_set_epi32($k3 as i32, $k2 as i32, $k1 as i32, $k0 as i32),
                );
                state1 = _mm_sha256rnds2_epu32(state1, state0, msg_k);
                state0 = _mm_sha256rnds2_epu32(state0, state1, _mm_srli_si128(msg_k, 8));
            }};
        }

        // Rounds 0..15
        sha256_4rounds!(
            msg0,
            0xe9b5dba5u32,
            0xb5c0fbcfu32,
            0x71374491u32,
            0x428a2f98u32
        );
        sha256_4rounds!(
            msg1,
            0xab1c5ed5u32,
            0x923f82a4u32,
            0x59f111f1u32,
            0x3956c25bu32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        sha256_4rounds!(
            msg2,
            0x550c7dc3u32,
            0x243185beu32,
            0x12835b01u32,
            0xd807aa98u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0xc19bf174u32,
            0x9bdc06a7u32,
            0x80deb1feu32,
            0x72be5d74u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 16..31
        sha256_4rounds!(
            msg0,
            0x240ca1ccu32,
            0x0fc19dc6u32,
            0xefbe4786u32,
            0xe49b69c1u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x76f988dau32,
            0x5cb0a9dcu32,
            0x4a7484aau32,
            0x2de92c6fu32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0xbf597fc7u32,
            0xb00327c8u32,
            0xa831c66du32,
            0x983e5152u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0x14292967u32,
            0x06ca6351u32,
            0xd5a79147u32,
            0xc6e00bf3u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 32..47
        sha256_4rounds!(
            msg0,
            0x53380d13u32,
            0x4d2c6dfcu32,
            0x2e1b2138u32,
            0x27b70a85u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x92722c85u32,
            0x81c2c92eu32,
            0x766a0abbu32,
            0x650a7354u32
        );
        msg0 = _mm_sha256msg1_epu32(msg0, msg1);
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0xc76c51a3u32,
            0xc24b8b70u32,
            0xa81a664bu32,
            0xa2bfe8a1u32
        );
        msg1 = _mm_sha256msg1_epu32(msg1, msg2);
        msg0 = _mm_add_epi32(msg0, _mm_alignr_epi8(msg3, msg2, 4));
        msg0 = _mm_sha256msg2_epu32(msg0, msg3);
        sha256_4rounds!(
            msg3,
            0x106aa070u32,
            0xf40e3585u32,
            0xd6990624u32,
            0xd192e819u32
        );
        msg2 = _mm_sha256msg1_epu32(msg2, msg3);
        msg1 = _mm_add_epi32(msg1, _mm_alignr_epi8(msg0, msg3, 4));
        msg1 = _mm_sha256msg2_epu32(msg1, msg0);

        // Rounds 48..63
        sha256_4rounds!(
            msg0,
            0x34b0bcb5u32,
            0x2748774cu32,
            0x1e376c08u32,
            0x19a4c116u32
        );
        msg3 = _mm_sha256msg1_epu32(msg3, msg0);
        msg2 = _mm_add_epi32(msg2, _mm_alignr_epi8(msg1, msg0, 4));
        msg2 = _mm_sha256msg2_epu32(msg2, msg1);
        sha256_4rounds!(
            msg1,
            0x682e6ff3u32,
            0x5b9cca4fu32,
            0x4ed8aa4au32,
            0x391c0cb3u32
        );
        msg3 = _mm_add_epi32(msg3, _mm_alignr_epi8(msg2, msg1, 4));
        msg3 = _mm_sha256msg2_epu32(msg3, msg2);
        sha256_4rounds!(
            msg2,
            0x8cc70208u32,
            0x84c87814u32,
            0x78a5636fu32,
            0x748f82eeu32
        );
        sha256_4rounds!(
            msg3,
            0xc67178f2u32,
            0xbef9a3f7u32,
            0xa4506cebu32,
            0x90befffau32
        );

        state0 = _mm_add_epi32(state0, abef_save);
        state1 = _mm_add_epi32(state1, cdgh_save);

        let out_tmp = _mm_shuffle_epi32(state0, 0x1B);
        let out_s1 = _mm_shuffle_epi32(state1, 0xB1);
        let out0 = _mm_blend_epi16(out_tmp, out_s1, 0xF0);
        let out1 = _mm_alignr_epi8(out_s1, out_tmp, 8);

        _mm_storeu_si128(h.as_mut_ptr() as *mut __m128i, out0);
        _mm_storeu_si128(h.as_mut_ptr().add(4) as *mut __m128i, out1);
    }

    #[inline]
    pub(crate) fn compress_block_soft(h: &mut [u32; 8], chunk: &[u8]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
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
    pub(crate) fn finalize(mut self) -> [u8; 32] {
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

        let mut out = [0u8; 32];
        for (idx, word) in self.h.iter().enumerate() {
            let bytes = word.to_be_bytes();
            out[idx * 4] = bytes[0];
            out[idx * 4 + 1] = bytes[1];
            out[idx * 4 + 2] = bytes[2];
            out[idx * 4 + 3] = bytes[3];
        }
        out
    }
}

/// Pure-Rust FIPS-180-4 SHA-256 implementation with zero intermediate heap allocations.
#[inline]
pub(crate) fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut state = Sha256State::new();
    state.update(data);
    state.finalize()
}

#[inline]
pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256_bytes(data);
    digest_to_hex(&digest)
}

/// RFC 2104 HMAC-SHA256 raw 32-byte digest with zero heap allocation.
#[inline]
pub(crate) fn hmac_sha256_bytes(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hashed_key = sha256_bytes(key);
        key_block[..32].copy_from_slice(&hashed_key);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Sha256State::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256State::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

/// RFC 2104 HMAC-SHA256 implementation streaming directly from borrowed slices with zero heap allocation.
#[inline]
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let digest = hmac_sha256_bytes(key, message);
    digest_to_hex(&digest)
}

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

#[inline]
pub(crate) fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut st = Sha1State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_sha1_bytes(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hk = sha1_bytes(key);
        key_block[..20].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha1State::new();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize();
    let mut outer = Sha1State::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}

/// Zero-allocation FIPS 180-4 SHA-512 / SHA-384 state machine.
pub(crate) struct Sha512State {
    h: [u64; 8],
    buf: [u8; 128],
    buf_len: usize,
    total_len: u128,
}

impl Sha512State {
    #[inline]
    pub(crate) fn new_512() -> Self {
        Self {
            h: [
                0x6a09e667f3bcc908,
                0xbb67ae8584caa73b,
                0x3c6ef372fe94f82b,
                0xa54ff53a5f1d36f1,
                0x510e527fade682d1,
                0x9b05688c2b3e6c1f,
                0x1f83d9abfb41bd6b,
                0x5be0cd19137e2179,
            ],
            buf: [0u8; 128],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    pub(crate) fn new_384() -> Self {
        Self {
            h: [
                0xcbbb9d5dc1059ed8,
                0x629a292a367cd507,
                0x9159015a3070dd17,
                0x152fecd8f70e5939,
                0x67332667ffc00b31,
                0x8eb44a8768581511,
                0xdb0c2e0d64f98fa7,
                0x47b5481dbefa4fa4,
            ],
            buf: [0u8; 128],
            buf_len: 0,
            total_len: 0,
        }
    }

    #[inline]
    fn compress_block(h: &mut [u64; 8], chunk: &[u8]) {
        const K: [u64; 80] = [
            0x428a2f98d728ae22,
            0x7137449123ef65cd,
            0xb5c0fbcfec4d3b2f,
            0xe9b5dba58189dbbc,
            0x3956c25bf348b538,
            0x59f111f1b605d019,
            0x923f82a4af194f9b,
            0xab1c5ed5da6d8118,
            0xd807aa98a3030242,
            0x12835b0145706fbe,
            0x243185be4ee4b28c,
            0x550c7dc3d5ffb4e2,
            0x72be5d74f27b896f,
            0x80deb1fe3b1696b1,
            0x9bdc06a725c71235,
            0xc19bf174cf692694,
            0xe49b69c19ef14ad2,
            0xefbe4786384f25e3,
            0x0fc19dc68b8cd5b5,
            0x240ca1cc77ac9c65,
            0x2de92c6f592b0275,
            0x4a7484aa6ea6e483,
            0x5cb0a9dcbd41fbd4,
            0x76f988da831153b5,
            0x983e5152ee66dfab,
            0xa831c66d2db43210,
            0xb00327c898fb213f,
            0xbf597fc7beef0ee4,
            0xc6e00bf33da88fc2,
            0xd5a79147930aa725,
            0x06ca6351e003826f,
            0x142929670a0e6e70,
            0x27b70a8546d22ffc,
            0x2e1b21385c26c926,
            0x4d2c6dfc5ac42aed,
            0x53380d139d95b3df,
            0x650a73548baf63de,
            0x766a0abb3c77b2a8,
            0x81c2c92e47edaee6,
            0x92722c851482353b,
            0xa2bfe8a14cf10364,
            0xa81a664bbc423001,
            0xc24b8b70d0f89791,
            0xc76c51a30654be30,
            0xd192e819d6ef5218,
            0xd69906245565a910,
            0xf40e35855771202a,
            0x106aa07032bbd1b8,
            0x19a4c116b8d2d0c8,
            0x1e376c085141ab53,
            0x2748774cdf8eeb99,
            0x34b0bcb5e19b48a8,
            0x391c0cb3c5c95a63,
            0x4ed8aa4ae3418acb,
            0x5b9cca4f7763e373,
            0x682e6ff3d6b2b8a3,
            0x748f82ee5defb2fc,
            0x78a5636f43172f60,
            0x84c87814a1f0ab72,
            0x8cc702081a6439ec,
            0x90befffa23631e28,
            0xa4506cebde82bde9,
            0xbef9a3f7b2c67915,
            0xc67178f2e372532b,
            0xca273eceea26619c,
            0xd186b8c721c0c207,
            0xeada7dd6cde0eb1e,
            0xf57d4f7fee6ed178,
            0x06f067aa72176fba,
            0x0a637dc5a2c898a6,
            0x113f9804bef90dae,
            0x1b710b35131c471b,
            0x28db77f523047d84,
            0x32caab7b40c72493,
            0x3c9ebe0a15c9bebc,
            0x431d67c49c100d4c,
            0x4cc5d4becb3e42b6,
            0x597f299cfc657e2a,
            0x5fcb6fab3ad6faec,
            0x6c44198c4a475817,
        ];

        let mut w = [0u64; 80];
        for (i, w_i) in w.iter_mut().enumerate().take(16) {
            let off = i * 8;
            *w_i = u64::from_be_bytes([
                chunk[off],
                chunk[off + 1],
                chunk[off + 2],
                chunk[off + 3],
                chunk[off + 4],
                chunk[off + 5],
                chunk[off + 6],
                chunk[off + 7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);

        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    #[inline]
    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u128;
        if self.buf_len > 0 {
            let need = 128 - self.buf_len;
            if data.len() < need {
                self.buf[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buf[self.buf_len..128].copy_from_slice(&data[..need]);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf_len = 0;
            data = &data[need..];
        }
        let (chunks, rem) = data.as_chunks::<128>();
        for chunk in chunks {
            Self::compress_block(&mut self.h, chunk);
        }
        if !rem.is_empty() {
            self.buf[..rem.len()].copy_from_slice(rem);
            self.buf_len = rem.len();
        }
    }

    #[inline]
    pub(crate) fn finalize_512(mut self) -> [u8; 64] {
        let bit_len = self.total_len * 8;
        self.buf[self.buf_len] = 0x80;
        self.buf_len += 1;
        if self.buf_len > 112 {
            self.buf[self.buf_len..128].fill(0);
            Self::compress_block(&mut self.h, &self.buf);
            self.buf.fill(0);
        } else {
            self.buf[self.buf_len..112].fill(0);
        }
        self.buf[112..128].copy_from_slice(&bit_len.to_be_bytes());
        Self::compress_block(&mut self.h, &self.buf);
        let mut out = [0u8; 64];
        for (idx, word) in self.h.iter().enumerate() {
            out[idx * 8..idx * 8 + 8].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    #[inline]
    pub(crate) fn finalize_384(self) -> [u8; 48] {
        let full = self.finalize_512();
        let mut out = [0u8; 48];
        out.copy_from_slice(&full[..48]);
        out
    }
}

#[inline]
pub(crate) fn sha512_bytes(data: &[u8]) -> [u8; 64] {
    let mut st = Sha512State::new_512();
    st.update(data);
    st.finalize_512()
}

#[inline]
pub(crate) fn sha384_bytes(data: &[u8]) -> [u8; 48] {
    let mut st = Sha512State::new_384();
    st.update(data);
    st.finalize_384()
}

#[inline]
pub(crate) fn hmac_sha512_bytes(key: &[u8], message: &[u8]) -> [u8; 64] {
    let mut key_block = [0u8; 128];
    if key.len() > 128 {
        let hk = sha512_bytes(key);
        key_block[..64].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 128];
    let mut opad = [0x5cu8; 128];
    for i in 0..128 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha512State::new_512();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize_512();
    let mut outer = Sha512State::new_512();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize_512()
}

#[inline]
pub(crate) fn hmac_sha384_bytes(key: &[u8], message: &[u8]) -> [u8; 48] {
    let mut key_block = [0u8; 128];
    if key.len() > 128 {
        let hk = sha384_bytes(key);
        key_block[..48].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 128];
    let mut opad = [0x5cu8; 128];
    for i in 0..128 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha512State::new_384();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize_384();
    let mut outer = Sha512State::new_384();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize_384()
}

/// Constant-time slice equality check for `crypto.subtle.verify` and `crypto.subtle.timingSafeEqual`.
#[inline]
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= std::hint::black_box(x ^ y);
    }
    std::hint::black_box(diff) == 0
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
            7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14,
            20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11,
            16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
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
            *item = u32::from_le_bytes([chunk[off], chunk[off + 1], chunk[off + 2], chunk[off + 3]]);
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

#[inline]
pub(crate) fn md5_bytes(data: &[u8]) -> [u8; 16] {
    let mut st = Md5State::new();
    st.update(data);
    st.finalize()
}

#[inline]
pub(crate) fn hmac_md5_bytes(key: &[u8], message: &[u8]) -> [u8; 16] {
    let mut key_block = [0u8; 64];
    if key.len() > 64 {
        let hk = md5_bytes(key);
        key_block[..16].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Md5State::new();
    inner.update(&ipad);
    inner.update(message);
    let ih = inner.finalize();
    let mut outer = Md5State::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}
