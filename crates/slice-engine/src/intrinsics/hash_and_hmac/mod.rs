//! Zero-Allocation Streaming Hash & HMAC State Machines (`SHA-NI` + FIPS 180-4 SHA-256,
//! SHA-1, SHA-384, SHA-512, RFC 1321 MD5, and Constant-Time Slice Equality).

pub(crate) mod sha1_and_md5;
#[allow(unsafe_code)]
pub mod sha256;
pub(crate) mod sha512;

pub(crate) use sha1_and_md5::*;
pub use sha256::*;
pub(crate) use sha512::*;

/// Const-generic streaming block hasher trait enabling a single zero-allocation RFC 2104 `HMAC` implementation.
pub(crate) trait BlockHasher<const BLOCK: usize, const OUT: usize> {
    fn init() -> Self;
    fn feed(&mut self, data: &[u8]);
    fn finish(self) -> [u8; OUT];
}

/// Computes an RFC 2104 `HMAC` digest over `(key, message)` on the stack with zero heap allocations.
#[inline(always)]
pub(crate) fn hmac_compute<H: BlockHasher<BLOCK, OUT>, const BLOCK: usize, const OUT: usize>(
    key: &[u8],
    message: &[u8],
) -> [u8; OUT] {
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let mut kh = H::init();
        kh.feed(key);
        let hk = kh.finish();
        key_block[..OUT].copy_from_slice(&hk);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = H::init();
    inner.feed(&ipad);
    inner.feed(message);
    let ih = inner.finish();

    let mut outer = H::init();
    outer.feed(&opad);
    outer.feed(&ih);
    outer.finish()
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
