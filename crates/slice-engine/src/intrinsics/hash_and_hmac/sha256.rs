//! Zero-Allocation Streaming FIPS 180-4 `SHA-256` State Machine with x86 `SHA-NI` Hardware Acceleration.

use super::{BlockHasher, hmac_compute};
use crate::intrinsics::encoding::digest_to_hex;

/// Zero-allocation streaming FIPS 180-4 `SHA-256` state machine.
/// Processes 64-byte chunks directly from borrowed `&[u8]` slices in place and buffers
/// only the final `<= 64` byte partial block on the stack (0 heap allocations).
pub struct Sha256State {
    pub(crate) h: [u32; 8],
    pub(crate) buf: [u8; 64],
    pub(crate) buf_len: usize,
    pub(crate) total_len: u64,
}

impl Sha256State {
    #[inline(always)]
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

    #[inline(always)]
    pub(crate) fn compress_block(h: &mut [u32; 8], chunk: &[u8; 64]) {
        if !Self::try_compress_block_sha_ni(h, chunk) {
            Self::compress_block_soft(h, chunk);
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[cold]
    #[inline(never)]
    fn detect_and_cache_sha_ni(slot: &std::sync::atomic::AtomicU8) -> bool {
        let detected = std::arch::is_x86_feature_detected!("sha")
            && std::arch::is_x86_feature_detected!("sse4.1")
            && std::arch::is_x86_feature_detected!("ssse3");
        slot.store(
            if detected { 1 } else { 2 },
            std::sync::atomic::Ordering::Relaxed,
        );
        detected
    }

    /// Compresses `chunk` into `h` using x86 `SHA-NI` instructions if supported by the host CPU,
    /// returning `true` on hardware execution or `false` (leaving `h` untouched) when unavailable.
    #[inline(always)]
    pub fn try_compress_block_sha_ni(h: &mut [u32; 8], chunk: &[u8; 64]) -> bool {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            static SHA_NI_STATUS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let status = SHA_NI_STATUS.load(std::sync::atomic::Ordering::Relaxed);
            let use_sha_ni = if status == 1 {
                true
            } else if status == 2 {
                false
            } else {
                Self::detect_and_cache_sha_ni(&SHA_NI_STATUS)
            };
            if use_sha_ni {
                // SAFETY: `sha`, `sse4.1`, and `ssse3` CPU features were verified via `is_x86_feature_detected!`.
                unsafe {
                    Self::compress_block_sha_ni(h, chunk);
                }
                return true;
            }
        }
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        {
            let _ = (h, chunk);
        }
        false
    }

    /// Compresses a single 64-byte SHA-256 block using x86 `SHA-NI` instructions.
    ///
    /// # Safety
    /// Caller must guarantee that the host CPU supports `sha`, `sse4.1`, and `ssse3`.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "sha,sse4.1,ssse3")]
    unsafe fn compress_block_sha_ni(h: &mut [u32; 8], chunk: &[u8; 64]) {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::*;
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::*;

        let shuf_mask = _mm_set_epi64x(0x0c0d0e0f08090a0b, 0x0405060700010203);

        let (h_lo, h_hi) = h.split_at(4);
        // SAFETY: `h_lo` and `h_hi` each hold 4 `u32` words (16 contiguous bytes), valid for unaligned 128-bit loads.
        let (state0_in, state1_in) = unsafe {
            (
                _mm_loadu_si128(h_lo.as_ptr().cast::<__m128i>()),
                _mm_loadu_si128(h_hi.as_ptr().cast::<__m128i>()),
            )
        };

        let tmp = _mm_shuffle_epi32(state0_in, 0xB1);
        let state1_shuf = _mm_shuffle_epi32(state1_in, 0x1B);
        let mut state0 = _mm_alignr_epi8(tmp, state1_shuf, 8);
        let mut state1 = _mm_blend_epi16(state1_shuf, tmp, 0xF0);

        let abef_save = state0;
        let cdgh_save = state1;

        let (lanes, _) = chunk.as_chunks::<16>();
        // SAFETY: `chunk` is `&[u8; 64]`, so `lanes` has 4 elements of 16 contiguous bytes each.
        let (raw0, raw1, raw2, raw3) = unsafe {
            (
                _mm_loadu_si128(lanes[0].as_ptr().cast::<__m128i>()),
                _mm_loadu_si128(lanes[1].as_ptr().cast::<__m128i>()),
                _mm_loadu_si128(lanes[2].as_ptr().cast::<__m128i>()),
                _mm_loadu_si128(lanes[3].as_ptr().cast::<__m128i>()),
            )
        };
        let mut msg0 = _mm_shuffle_epi8(raw0, shuf_mask);
        let mut msg1 = _mm_shuffle_epi8(raw1, shuf_mask);
        let mut msg2 = _mm_shuffle_epi8(raw2, shuf_mask);
        let mut msg3 = _mm_shuffle_epi8(raw3, shuf_mask);

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

        let (h_lo_mut, h_hi_mut) = h.split_at_mut(4);
        // SAFETY: `h_lo_mut` and `h_hi_mut` are disjoint mutable 16-byte slices valid for unaligned 128-bit stores.
        unsafe {
            _mm_storeu_si128(h_lo_mut.as_mut_ptr().cast::<__m128i>(), out0);
            _mm_storeu_si128(h_hi_mut.as_mut_ptr().cast::<__m128i>(), out1);
        }
    }

    #[inline]
    pub(crate) fn compress_block_soft(h: &mut [u32; 8], chunk: &[u8; 64]) {
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

    #[inline(always)]
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

    #[inline(always)]
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

impl BlockHasher<64, 32> for Sha256State {
    #[inline(always)]
    fn init() -> Self {
        Self::new()
    }

    #[inline(always)]
    fn feed(&mut self, data: &[u8]) {
        self.update(data);
    }

    #[inline(always)]
    fn finish(self) -> [u8; 32] {
        self.finalize()
    }
}

/// Pure-Rust FIPS 180-4 SHA-256 implementation with zero intermediate heap allocations.
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
    hmac_compute::<Sha256State, 64, 32>(key, message)
}

/// RFC 2104 HMAC-SHA256 implementation streaming directly from borrowed slices with zero heap allocation.
#[inline]
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let digest = hmac_sha256_bytes(key, message);
    digest_to_hex(&digest)
}
