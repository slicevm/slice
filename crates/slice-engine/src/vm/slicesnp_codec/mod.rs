//! Binary `SLICESNP\x02` Snapshot Encoder/Decoder and Content-Verified Module Cache.
//!
//! Replaces verbose `serde_json` `BytecodeModule` snapshots with a compact bounds-checked
//! binary wire format (`b"SLICESNP\x02"`), while preserving backward compatibility with legacy
//! `SLICESNP{...}` JSON snapshots.
//!
//! Also eliminates the `SNP_LAST_HIT` `(ptr, len)` `mimalloc` LIFO pointer-reuse ABA hazard by
//! storing an immutable `Arc<[u8]>` copy of the snapshot payload alongside each cached
//! `Arc<BytecodeModule>` and verifying exact byte equality on every cache lookup.

mod cache;
mod decoder;
mod encoder;

pub(crate) use cache::{
    decode_slicesnp_cached, decode_slicesnp_import_cached, visit_pure_static_exports,
};
pub(crate) use decoder::SnpCursor;
pub use decoder::decode_slicesnp_module;
pub use encoder::encode_slicesnp_v2;

use crate::value_and_ast::{BinOp, UnaryOp};

/// Binary wire format version byte placed immediately after `SNAPSHOT_MAGIC` (`b"SLICESNP"`).
pub const SLICESNP_V2_VERSION: u8 = 0x02;

/// Flag bit at `bytecode[9]` indicating the module consists strictly of numeric `static_exports`
/// (`ConstNum` ops matching `static_exports`), allowing zero-allocation `&str` export iteration.
pub const SLICESNP_FLAG_PURE_STATIC_EXPORTS: u8 = 0x01;

/// Flag bit at `bytecode[9]` indicating every instruction in `ops` is a flat register/host opcode
/// (no nested `EvalStmt` AST or `BoaFullEsVm`), enabling zero-copy `LLInt` cursor execution.
pub const SLICESNP_FLAG_FLAT_LLINT: u8 = 0x02;

/// Upper bound on per-thread cached `SLICESNP` modules (comfortably holds 500-module `.cas`
/// archives plus entrypoints without thrashing while strictly bounding thread-local memory).
pub const MAX_SNP_CACHE_ENTRIES: usize = 1024;

/// Maximum AST/value nesting depth accepted by the binary decoder so crafted recursive byte
/// streams fail closed without risking host stack overflow (including on 1 MB Windows debug stacks).
pub(super) const MAX_CODEC_DEPTH: u16 = if cfg!(debug_assertions) { 64 } else { 192 };

/// Maximum pre-allocated container capacity per decoded length prefix before reading elements,
/// preventing truncated payloads with huge `u32` prefixes from triggering OOM panics.
pub(super) const MAX_PREALLOC_ITEMS: usize = 4096;

#[inline]
pub(super) fn bin_op_to_u8(op: BinOp) -> u8 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::Div => 3,
        BinOp::Rem => 4,
        BinOp::Pow => 5,
        BinOp::Eq => 6,
        BinOp::Neq => 7,
        BinOp::StrictEq => 8,
        BinOp::StrictNeq => 9,
        BinOp::Lt => 10,
        BinOp::Lte => 11,
        BinOp::Gt => 12,
        BinOp::Gte => 13,
        BinOp::And => 14,
        BinOp::Or => 15,
        BinOp::BitAnd => 16,
        BinOp::BitOr => 17,
        BinOp::BitXor => 18,
        BinOp::Shl => 19,
        BinOp::Shr => 20,
        BinOp::UShr => 21,
    }
}

#[inline]
pub(super) fn u8_to_bin_op(b: u8) -> Option<BinOp> {
    Some(match b {
        0 => BinOp::Add,
        1 => BinOp::Sub,
        2 => BinOp::Mul,
        3 => BinOp::Div,
        4 => BinOp::Rem,
        5 => BinOp::Pow,
        6 => BinOp::Eq,
        7 => BinOp::Neq,
        8 => BinOp::StrictEq,
        9 => BinOp::StrictNeq,
        10 => BinOp::Lt,
        11 => BinOp::Lte,
        12 => BinOp::Gt,
        13 => BinOp::Gte,
        14 => BinOp::And,
        15 => BinOp::Or,
        16 => BinOp::BitAnd,
        17 => BinOp::BitOr,
        18 => BinOp::BitXor,
        19 => BinOp::Shl,
        20 => BinOp::Shr,
        21 => BinOp::UShr,
        _ => return None,
    })
}

#[inline]
pub(super) fn unary_op_to_u8(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Neg => 0,
        UnaryOp::Pos => 1,
        UnaryOp::Not => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::TypeOf => 4,
    }
}

#[inline]
pub(super) fn u8_to_unary_op(b: u8) -> Option<UnaryOp> {
    Some(match b {
        0 => UnaryOp::Neg,
        1 => UnaryOp::Pos,
        2 => UnaryOp::Not,
        3 => UnaryOp::BitNot,
        4 => UnaryOp::TypeOf,
        _ => return None,
    })
}
