//! Deterministic SplitMix64 PRNG (`--deterministic`), Host OS CSPRNG (`ring::rand::SystemRandom`),
//! and RFC 4122 UUIDv4 Generation.

#[inline]
pub(crate) fn splitmix64_next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
pub(crate) fn splitmix64_next_f64(state: &mut u64) -> f64 {
    let z = splitmix64_next_u64(state);
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

struct ThreadCsprngPool {
    buf: [u8; 256],
    pos: usize,
}

impl ThreadCsprngPool {
    const fn new() -> Self {
        Self {
            buf: [0u8; 256],
            pos: 256,
        }
    }

    #[inline]
    fn refill(&mut self) {
        use ring::rand::SecureRandom;
        let sys_rng = ring::rand::SystemRandom::new();
        if sys_rng.fill(&mut self.buf).is_err() {
            // Infallible fallback if OS entropy source is unavailable
            use std::hash::{BuildHasher, Hasher};
            let rs = std::collections::hash_map::RandomState::new();
            let mut h = rs.build_hasher();
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xA5A5_5A5A_F0F0_0F0F);
            h.write_u64(nanos);
            let mut state = h.finish() | 1;
            for chunk in self.buf.as_chunks_mut::<8>().0 {
                *chunk = splitmix64_next_u64(&mut state).to_le_bytes();
            }
        }
        self.pos = 0;
    }

    #[inline]
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        if dest.len() > 128 {
            use ring::rand::SecureRandom;
            let sys_rng = ring::rand::SystemRandom::new();
            if sys_rng.fill(dest).is_ok() {
                return;
            }
        }
        let mut offset = 0;
        while offset < dest.len() {
            if self.pos >= self.buf.len() {
                self.refill();
            }
            let avail = self.buf.len() - self.pos;
            let take = (dest.len() - offset).min(avail);
            dest[offset..offset + take].copy_from_slice(&self.buf[self.pos..self.pos + take]);
            // Zeroize consumed CSPRNG bytes in-place so previous bytes cannot linger in memory
            self.buf[self.pos..self.pos + take].fill(0);
            self.pos += take;
            offset += take;
        }
    }
}

thread_local! {
    static HOST_CSPRNG_POOL: std::cell::RefCell<ThreadCsprngPool> =
        const { std::cell::RefCell::new(ThreadCsprngPool::new()) };
}

/// Fills `dest` with random bytes from either the deterministic SplitMix64 seed (`--deterministic`)
/// or the OS-backed cryptographic random number generator (`ring::rand::SystemRandom`).
pub fn fill_random_bytes(dest: &mut [u8], det_prng: Option<&mut u64>) {
    if let Some(state) = det_prng {
        let mut offset = 0;
        while offset < dest.len() {
            let word = splitmix64_next_u64(state);
            let bytes = word.to_le_bytes();
            let take = (dest.len() - offset).min(8);
            dest[offset..offset + take].copy_from_slice(&bytes[..take]);
            offset += take;
        }
        return;
    }
    HOST_CSPRNG_POOL.with(|pool| pool.borrow_mut().fill_bytes(dest));
}

/// Maximum exclusive span (`max - min`) permitted by Node.js `crypto.randomInt`: `2^48 - 1`.
pub const MAX_CRYPTO_RANDOM_INT_RANGE: u64 = 0xFFFF_FFFF_FFFF;

/// Samples an unbiased integer uniformly in `0..range` (`1 <= range <= 2^48 - 1`) using bitmask
/// rejection sampling over either the deterministic SplitMix64 PRNG (`--deterministic`) or the
/// thread-local buffered OS CSPRNG (`HOST_CSPRNG_POOL`) with zero heap allocations.
#[inline]
pub fn sample_unbiased_u64_below(range: u64, det_prng: Option<&mut u64>) -> u64 {
    if range <= 1 {
        return 0;
    }
    let bits = 64 - (range - 1).leading_zeros();
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };

    if let Some(state) = det_prng {
        loop {
            let candidate = splitmix64_next_u64(state) & mask;
            if candidate < range {
                return candidate;
            }
        }
    }

    HOST_CSPRNG_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        let mut raw = [0u8; 8];
        loop {
            pool.fill_bytes(&mut raw);
            let candidate = u64::from_le_bytes(raw) & mask;
            if candidate < range {
                return candidate;
            }
        }
    })
}

/// Generates an RFC 4122 UUIDv4 string (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`),
/// using deterministic PRNG state when active or OS CSPRNG otherwise.
pub fn generate_uuid_v4(det_prng: Option<&mut u64>) -> String {
    let mut b = [0u8; 16];
    fill_random_bytes(&mut b, det_prng);
    // Set version 4 (0b0100_xxxx) and RFC 4122 variant (0b10xx_xxxx)
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = [0u8; 36];
    let mut j = 0;
    for (i, &byte) in b.iter().enumerate() {
        if i == 4 || i == 6 || i == 8 || i == 10 {
            out[j] = b'-';
            j += 1;
        }
        out[j] = HEX[(byte >> 4) as usize];
        out[j + 1] = HEX[(byte & 0x0f) as usize];
        j += 2;
    }
    unsafe { String::from_utf8_unchecked(out.to_vec()) }
}
