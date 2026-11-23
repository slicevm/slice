//! Tenant-Isolated Pre-Expanded Cryptographic Key Handle Table (`CachedCryptoKeyMaterial`).

use crate::intrinsics::symmetric_and_kdf::AesExpandedKey;

/// Cached pre-expanded cryptographic key entry stored in the isolate-local / tenant-scoped slot table.
#[allow(dead_code, clippy::large_enum_variant)]
pub(crate) enum CachedCryptoKeyMaterial {
    AesGcm {
        less_safe: ring::aead::LessSafeKey,
        raw: Vec<u8>,
    },
    AesCbc {
        expanded: AesExpandedKey,
        raw: Vec<u8>,
    },
    Hmac {
        ring_key: ring::hmac::Key,
        hash_alg: String,
        raw: Vec<u8>,
    },
    Ed25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    Ed25519Public {
        pub_bytes: [u8; 32],
    },
    X25519Private {
        seed: [u8; 32],
        pub_bytes: [u8; 32],
    },
    X25519Public {
        pub_bytes: [u8; 32],
    },
    EcdsaPrivate {
        curve: String,
        pkcs8: Vec<u8>,
        d_bytes: Vec<u8>,
        pub_bytes: Vec<u8>,
    },
    EcdsaPublic {
        curve: String,
        pub_bytes: Vec<u8>,
    },
    RsaPrivate {
        pkcs8: Vec<u8>,
        spki: Vec<u8>,
    },
    RsaPublic {
        spki: Vec<u8>,
    },
}

pub(crate) struct TenantCachedCryptoKey {
    pub(crate) tenant_id: String,
    pub(crate) material: CachedCryptoKeyMaterial,
}

thread_local! {
    static NEXT_CRYPTO_KEY_HANDLE: std::cell::Cell<u32> = const { std::cell::Cell::new(1) };
    static CRYPTO_KEY_HANDLE_TABLE: std::cell::RefCell<rustc_hash::FxHashMap<u32, TenantCachedCryptoKey>> =
        std::cell::RefCell::new(rustc_hash::FxHashMap::default());
}

/// Inserts a pre-expanded `CachedCryptoKeyMaterial` into the tenant-scoped handle table, returning its opaque `u32` ID.
pub(crate) fn insert_cached_crypto_key(
    tenant_id: String,
    material: CachedCryptoKeyMaterial,
) -> u32 {
    let id = NEXT_CRYPTO_KEY_HANDLE.with(|c| {
        let cur = c.get();
        c.set(cur.wrapping_add(1).max(1));
        cur
    });
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let mut borrow = table.borrow_mut();
        if borrow.len() >= 1024 {
            borrow.retain(|_, v| v.tenant_id == tenant_id);
            if borrow.len() >= 1024 {
                borrow.clear();
            }
        }
        borrow.insert(
            id,
            TenantCachedCryptoKey {
                tenant_id,
                material,
            },
        );
    });
    id
}

/// Executes `f` with a reference to the cached `CachedCryptoKeyMaterial` for `handle_id` ONLY if its
/// `tenant_id` matches `expected_tenant`, enforcing strict multi-tenant key isolation.
#[inline]
pub(crate) fn with_cached_crypto_key<R>(
    handle_id: u32,
    expected_tenant: &str,
    f: impl FnOnce(&CachedCryptoKeyMaterial) -> R,
) -> Option<R> {
    if handle_id == 0 {
        return None;
    }
    CRYPTO_KEY_HANDLE_TABLE.with(|table| {
        let borrow = table.borrow();
        let entry = borrow.get(&handle_id)?;
        if entry.tenant_id != expected_tenant {
            return None;
        }
        Some(f(&entry.material))
    })
}
