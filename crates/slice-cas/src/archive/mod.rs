//! Zero-Stat Content-Addressed Storage (`.cas`) Binary Archive & Static Asset Subsystem.
//!
//! Organizes the binary `.cas` archive layer into focused submodules:
//! - [`builder`]: [`CasBuilder`] for packing JS/TS modules, static web assets, and `slice.toml` manifests.
//! - [`reader`]: [`MappedCasArchive`] zero-copy `mmap` reader and `SLICEEXE` standalone binary packer.
//! - [`static_assets`]: [`StaticAssetTable`], [`StaticAssetEntry`], and BLAKE3 `ETag` negotiation.

pub mod builder;
pub mod reader;
pub mod static_assets;

pub use builder::CasBuilder;
pub use reader::{embed_cas_into_executable, MappedCasArchive};
pub use static_assets::{
    detect_static_mime_type, StaticAssetEntry, StaticAssetPayload, StaticAssetTable,
};

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

pub const CAS_MAGIC: &[u8; 8] = b"SLICECAS";
pub const CAS_VERSION: u32 = 1;
pub const CACHE_LINE_ALIGN: usize = 64;

/// 32-byte BLAKE3 cryptographic hash identifying a canonicalized module blob.
pub type Blake3Hash = [u8; 32];

/// Metadata entry in the CAS Radix/Index table pointing to a 64-byte aligned slice.
#[repr(C)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasBlobEntry {
    pub offset: u64,
    pub source_len: u64,
    pub bytecode_offset: u64,
    pub bytecode_len: u64,
    pub is_typescript: bool,
    pub hash_hex: String,
    pub original_specifier: String,
}

/// Deterministic Radix / Hash resolution table embedded in the CAS header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CasIndexHeader {
    pub version: u32,
    pub entrypoint_specifier: String,
    pub entrypoint_hash: String,
    /// Maps `(issuer_context, import_specifier)` or canonical specifier -> `hash_hex`
    pub radix_resolution_map: FxHashMap<String, String>,
    /// Maps `hash_hex` -> `CasBlobEntry`
    pub blobs: FxHashMap<String, CasBlobEntry>,
    /// Compact source-line table (`(compacted_line_1, "#slice-source payload")`) for `.cas` and `SLICEEXE` stack traces
    #[serde(default)]
    pub source_map_markers: Vec<(usize, String)>,
}
