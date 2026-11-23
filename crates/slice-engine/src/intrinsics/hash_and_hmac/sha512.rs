//! Zero-Allocation Streaming FIPS 180-4 `SHA-512` and `SHA-384` State Machine & `HMAC`.

use super::{BlockHasher, hmac_compute};

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

impl BlockHasher<128, 64> for Sha512State {
    #[inline(always)]
    fn init() -> Self {
        Self::new_512()
    }

    #[inline(always)]
    fn feed(&mut self, data: &[u8]) {
        self.update(data);
    }

    #[inline(always)]
    fn finish(self) -> [u8; 64] {
        self.finalize_512()
    }
}

struct Sha384Hasher(Sha512State);

impl BlockHasher<128, 48> for Sha384Hasher {
    #[inline(always)]
    fn init() -> Self {
        Self(Sha512State::new_384())
    }

    #[inline(always)]
    fn feed(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    #[inline(always)]
    fn finish(self) -> [u8; 48] {
        self.0.finalize_384()
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
    hmac_compute::<Sha512State, 128, 64>(key, message)
}

#[inline]
pub(crate) fn hmac_sha384_bytes(key: &[u8], message: &[u8]) -> [u8; 48] {
    hmac_compute::<Sha384Hasher, 128, 48>(key, message)
}
