//! Native Rust SIMD & Fast-Path Intrinsics (`SHA-NI` / FIPS-180-4 SHA-256, HMAC, AES-CBC/GCM,
//! PBKDF2/HKDF/scrypt, Ed25519/ECDSA, Base64/Hex/WHATWG URL/USP, DEFLATE/Zlib/Gzip, SemVer, `ms`, `bytes`).
//!
//! Organized into focused domain submodules:
//! - [`fast_parsers`]: Zero-allocation `ms`, `bytes`, and `SemVer` parsers/formatters.
//! - [`encoding`]: Fast lookup-table Hex, Base64, Base64URL, and strict WHATWG `btoa` / `atob` codecs.
//! - [`url_and_usp`]: Native WHATWG `URL`, `URL.canParse`, `URLSearchParams`, and pathname normalizer.
//! - [`hash_and_hmac`]: Streaming `SHA-256` (`SHA-NI`), `SHA-1`, `SHA-384`, `SHA-512`, `MD5`, `HMAC`, and `constant_time_eq`.
//! - [`symmetric_and_kdf`]: Branchless `AES-128/256-CBC`, `ring` `AES-128/256-GCM`, `PBKDF2`, `HKDF`, and `scrypt`.
//! - [`asymmetric_and_keys`]: `Ed25519`, `X25519`, `ECDSA P-256/P-384`, and `RSA 2048..8192` PKCS#8/PKCS#1/SPKI/JWK codecs and tenant-scoped key handle table.
//! - [`prng_and_uuid`]: Deterministic `SplitMix64` PRNG, OS-backed `SystemRandom` CSPRNG pool, and RFC 4122 `UUIDv4`.
//! - [`compression`]: Self-contained RFC 1951 `DEFLATE`, RFC 1950 `Zlib`, and RFC 1952 `Gzip` with 9-bit Huffman LUT and table CRC-32.

pub mod asymmetric_and_keys;
pub mod compression;
pub mod encoding;
pub mod fast_parsers;
pub mod hash_and_hmac;
pub mod prng_and_uuid;
pub mod symmetric_and_kdf;
pub mod url_and_usp;

pub use asymmetric_and_keys::*;
pub use compression::*;
pub use encoding::*;
pub use fast_parsers::*;
pub use hash_and_hmac::*;
pub use prng_and_uuid::*;
pub use symmetric_and_kdf::*;
pub use url_and_usp::{
    can_parse_whatwg_url, is_uri_unescaped, normalize_url_pathname, parse_whatwg_url,
    usp_decode_component, usp_encode_component, usp_parse_query, usp_stringify_pairs,
    ParsedWhatwgUrl,
};
