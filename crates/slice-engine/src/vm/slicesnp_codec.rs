//! Binary `SLICESNP\x02` Snapshot Encoder/Decoder and Content-Verified Module Cache.
//!
//! Replaces verbose `serde_json` `BytecodeModule` snapshots with a compact bounds-checked
//! binary wire format (`b"SLICESNP\x02"`), while preserving backward compatibility with legacy
//! `SLICESNP{...}` JSON snapshots.
//!
//! Also eliminates the `SNP_LAST_HIT` `(ptr, len)` `mimalloc` LIFO pointer-reuse ABA hazard by
//! storing an immutable `Arc<[u8]>` copy of the snapshot payload alongside each cached
//! `Arc<BytecodeModule>` and verifying exact byte equality on every cache lookup.

use crate::value_and_ast::{
    BinOp, BytecodeModule, BytecodeOp, Expr, JsFunction, JsValue, SNAPSHOT_MAGIC, Stmt, UnaryOp,
};
use anyhow::{Result, bail};
use rustc_hash::{FxHashMap, FxHasher};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

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
/// streams fail closed without risking host stack overflow.
const MAX_CODEC_DEPTH: u16 = 512;

/// Maximum pre-allocated container capacity per decoded length prefix before reading elements,
/// preventing truncated payloads with huge `u32` prefixes from triggering OOM panics.
const MAX_PREALLOC_ITEMS: usize = 4096;

#[derive(Clone)]
struct CachedSnpEntry {
    raw: Arc<[u8]>,
    module: Arc<BytecodeModule>,
}

thread_local! {
    static SNP_LAST_HIT: RefCell<Option<CachedSnpEntry>> = const { RefCell::new(None) };
    static SNP_MODULE_CACHE: RefCell<FxHashMap<u64, CachedSnpEntry>> =
        RefCell::new(FxHashMap::default());
}

#[inline]
fn empty_bytecode_module() -> Arc<BytecodeModule> {
    static EMPTY: OnceLock<Arc<BytecodeModule>> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| {
        Arc::new(BytecodeModule {
            ops: Vec::new(),
            static_exports: FxHashMap::default(),
        })
    }))
}

#[inline]
fn fx_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = FxHasher::default();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Decodes and caches a top-level `SLICESNP` entrypoint bytecode slice, updating `SNP_LAST_HIT`
/// and verifying exact payload bytes (`entry.raw.as_ref() == &bytecode[8..]`) so equal-length
/// `mimalloc` buffer reuse at the same address never returns a stale `BytecodeModule`.
#[inline]
pub(crate) fn decode_slicesnp_cached(bytecode: &[u8]) -> Arc<BytecodeModule> {
    if bytecode.len() < 9 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        return empty_bytecode_module();
    }
    let payload = &bytecode[8..];
    if let Some(hit) = SNP_LAST_HIT.with(|slot| {
        let borrow = slot.borrow();
        if let Some(entry) = borrow.as_ref()
            && entry.raw.as_ref() == payload
        {
            Some(Arc::clone(&entry.module))
        } else {
            None
        }
    }) {
        return hit;
    }

    let entry = lookup_or_insert_snp_cache(bytecode, payload);
    let module = Arc::clone(&entry.module);
    SNP_LAST_HIT.with(|slot| {
        *slot.borrow_mut() = Some(entry);
    });
    module
}

/// Decodes and caches an imported child `SLICESNP` blob inside `BytecodeOp::ImportCas` without
/// displacing the active entrypoint in `SNP_LAST_HIT`.
#[inline]
pub(crate) fn decode_slicesnp_import_cached(bytecode: &[u8]) -> Option<Arc<BytecodeModule>> {
    if bytecode.len() < 9 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        return None;
    }
    let payload = &bytecode[8..];
    let key = fx_hash_bytes(payload);
    SNP_MODULE_CACHE.with(|cache| {
        if let Some(existing) = cache.borrow().get(&key)
            && existing.raw.as_ref() == payload
        {
            return Some(Arc::clone(&existing.module));
        }
        let parsed = decode_slicesnp_module(bytecode).ok()?;
        let arc_mod = Arc::new(parsed);
        let raw_arc: Arc<[u8]> = Arc::from(payload);
        let mut map = cache.borrow_mut();
        if map.len() >= MAX_SNP_CACHE_ENTRIES && !map.contains_key(&key) {
            map.clear();
        }
        map.insert(
            key,
            CachedSnpEntry {
                raw: raw_arc,
                module: Arc::clone(&arc_mod),
            },
        );
        Some(arc_mod)
    })
}

fn lookup_or_insert_snp_cache(bytecode: &[u8], payload: &[u8]) -> CachedSnpEntry {
    let key = fx_hash_bytes(payload);
    SNP_MODULE_CACHE.with(|cache| {
        if let Some(existing) = cache.borrow().get(&key)
            && existing.raw.as_ref() == payload
        {
            return existing.clone();
        }
        let arc_mod = match decode_slicesnp_module(bytecode) {
            Ok(parsed) => Arc::new(parsed),
            Err(_) => empty_bytecode_module(),
        };
        let entry = CachedSnpEntry {
            raw: Arc::from(payload),
            module: arc_mod,
        };
        let mut map = cache.borrow_mut();
        if map.len() >= MAX_SNP_CACHE_ENTRIES && !map.contains_key(&key) {
            map.clear();
        }
        map.insert(key, entry.clone());
        entry
    })
}

/// Zero-allocation iterator over `(export_name: &str, value: f64)` pairs inside a
/// `SLICESNP\x02` pure static-export snapshot (`SLICESNP_FLAG_PURE_STATIC_EXPORTS`).
pub(crate) struct PureStaticExportsIter<'a> {
    cur: SnpCursor<'a>,
    remaining_exports: usize,
}

impl<'a> Iterator for PureStaticExportsIter<'a> {
    type Item = (&'a str, f64);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_exports == 0 {
            return None;
        }
        self.remaining_exports -= 1;
        let k = self.cur.read_str()?;
        let v = self.cur.read_f64()?;
        Some((k, v))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining_exports, Some(self.remaining_exports))
    }
}

#[inline]
pub(crate) fn visit_pure_static_exports(bytecode: &[u8]) -> Option<PureStaticExportsIter<'_>> {
    if bytecode.len() < 14
        || &bytecode[0..8] != SNAPSHOT_MAGIC
        || bytecode[8] != SLICESNP_V2_VERSION
        || (bytecode[9] & SLICESNP_FLAG_PURE_STATIC_EXPORTS) == 0
    {
        return None;
    }
    let mut cur = SnpCursor::new(&bytecode[10..]);
    let count = cur.read_u32_len()?;
    Some(PureStaticExportsIter {
        cur,
        remaining_exports: count,
    })
}

#[inline]
fn is_pure_static_exports_module(module: &BytecodeModule) -> bool {
    !module.static_exports.is_empty()
        && module.ops.len() == module.static_exports.len()
        && module.ops.iter().all(|op| match op {
            BytecodeOp::ConstNum { dst, val } => module
                .static_exports
                .get(dst)
                .is_some_and(|exp_v| exp_v.to_bits() == val.to_bits()),
            _ => false,
        })
}

#[inline]
fn is_flat_llint_module(module: &BytecodeModule) -> bool {
    !module.ops.is_empty()
        && module
            .ops
            .iter()
            .all(|op| !matches!(op, BytecodeOp::EvalStmt(_) | BytecodeOp::BoaFullEsVm { .. }))
}

/// Encodes a [`BytecodeModule`] into a compact deterministic `SLICESNP\x02` binary snapshot.
pub fn encode_slicesnp_v2(module: &BytecodeModule) -> Vec<u8> {
    let pure_static = is_pure_static_exports_module(module);
    let flat_llint = is_flat_llint_module(module);
    let mut flags: u8 = 0;
    if pure_static {
        flags |= SLICESNP_FLAG_PURE_STATIC_EXPORTS;
    }
    if flat_llint {
        flags |= SLICESNP_FLAG_FLAT_LLINT;
    }

    let est_cap = if pure_static {
        32 + module.static_exports.len() * 20
    } else {
        32 + module.ops.len() * 20
    };
    let mut out = Vec::with_capacity(est_cap);
    out.extend_from_slice(SNAPSHOT_MAGIC);
    out.push(SLICESNP_V2_VERSION);
    out.push(flags);

    write_u32(&mut out, module.static_exports.len() as u32);
    if module.static_exports.len() <= 1 {
        for (k, v) in &module.static_exports {
            write_str(&mut out, k);
            write_f64(&mut out, *v);
        }
    } else {
        let mut ordered_exports: Vec<(&String, f64)> =
            module.static_exports.iter().map(|(k, v)| (k, *v)).collect();
        ordered_exports.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for &(k, v) in &ordered_exports {
            write_str(&mut out, k);
            write_f64(&mut out, v);
        }
    }

    if pure_static {
        write_u32(&mut out, 0);
    } else {
        write_u32(&mut out, module.ops.len() as u32);
        for op in &module.ops {
            encode_op(&mut out, op);
        }
    }
    out
}

/// Decodes either a binary `SLICESNP\x02` snapshot or a legacy `SLICESNP{...}` JSON snapshot.
pub fn decode_slicesnp_module(bytecode: &[u8]) -> Result<BytecodeModule> {
    if bytecode.len() < 9 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        bail!("Invalid SLICESNP bytecode header");
    }
    let version_or_json = bytecode[8];
    if version_or_json == SLICESNP_V2_VERSION {
        if bytecode.len() < 10 {
            bail!("Truncated SLICESNPv2 flags byte");
        }
        let flags = bytecode[9];
        if (flags & !(SLICESNP_FLAG_PURE_STATIC_EXPORTS | SLICESNP_FLAG_FLAT_LLINT)) != 0 {
            bail!("Unknown SLICESNPv2 flags: 0x{:02x}", flags);
        }
        let mut cur = SnpCursor::new(&bytecode[10..]);
        let export_count = cur
            .read_u32_len()
            .ok_or_else(|| anyhow::anyhow!("Truncated SLICESNPv2 static_exports length"))?;
        let mut static_exports = FxHashMap::with_capacity_and_hasher(
            export_count.min(MAX_PREALLOC_ITEMS),
            Default::default(),
        );
        let mut pure_ops = if (flags & SLICESNP_FLAG_PURE_STATIC_EXPORTS) != 0 {
            Vec::with_capacity(export_count.min(MAX_PREALLOC_ITEMS))
        } else {
            Vec::new()
        };
        for _ in 0..export_count {
            let k = cur
                .read_string()
                .ok_or_else(|| anyhow::anyhow!("Invalid SLICESNPv2 export key"))?;
            let v = cur
                .read_f64()
                .ok_or_else(|| anyhow::anyhow!("Truncated SLICESNPv2 export value"))?;
            if (flags & SLICESNP_FLAG_PURE_STATIC_EXPORTS) != 0 {
                pure_ops.push(BytecodeOp::ConstNum {
                    dst: k.clone(),
                    val: v,
                });
            }
            static_exports.insert(k, v);
        }

        let op_count = cur
            .read_u32_len()
            .ok_or_else(|| anyhow::anyhow!("Truncated SLICESNPv2 ops length"))?;
        let ops = if (flags & SLICESNP_FLAG_PURE_STATIC_EXPORTS) != 0 && op_count == 0 {
            pure_ops
        } else {
            let mut decoded_ops = Vec::with_capacity(op_count.min(MAX_PREALLOC_ITEMS));
            for _ in 0..op_count {
                let op = cur.decode_op(0).ok_or_else(|| {
                    anyhow::anyhow!("Corrupt or truncated SLICESNPv2 instruction")
                })?;
                decoded_ops.push(op);
            }
            decoded_ops
        };
        if !cur.is_empty() {
            bail!(
                "Trailing {} unexpected bytes in SLICESNPv2 payload",
                cur.remaining()
            );
        }
        Ok(BytecodeModule {
            ops,
            static_exports,
        })
    } else if version_or_json == b'{' {
        let module: BytecodeModule = serde_json::from_slice(&bytecode[8..])?;
        Ok(module)
    } else {
        bail!(
            "Unsupported SLICESNP snapshot version byte: 0x{:02x}",
            version_or_json
        )
    }
}

#[inline]
fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

#[inline]
fn write_bool(out: &mut Vec<u8>, v: bool) {
    out.push(u8::from(v));
}

#[inline]
fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[inline]
fn write_str(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

#[inline]
fn write_str_slice(out: &mut Vec<u8>, items: &[String]) {
    write_u32(out, items.len() as u32);
    for s in items {
        write_str(out, s);
    }
}

fn encode_op(out: &mut Vec<u8>, op: &BytecodeOp) {
    match op {
        BytecodeOp::ConstNum { dst, val } => {
            write_u8(out, 0);
            write_str(out, dst);
            write_f64(out, *val);
        }
        BytecodeOp::ConstStr { dst, val } => {
            write_u8(out, 1);
            write_str(out, dst);
            write_str(out, val);
        }
        BytecodeOp::ImportCas { dst, specifier } => {
            write_u8(out, 2);
            write_str(out, dst);
            write_str(out, specifier);
        }
        BytecodeOp::AddNum { dst, lhs, rhs } => {
            write_u8(out, 3);
            write_str(out, dst);
            write_str(out, lhs);
            write_str(out, rhs);
        }
        BytecodeOp::NumericLoopKernel { dst, iterations } => {
            write_u8(out, 4);
            write_str(out, dst);
            write_u64(out, *iterations);
        }
        BytecodeOp::FibKernel { dst, n } => {
            write_u8(out, 5);
            write_str(out, dst);
            write_u64(out, *n);
        }
        BytecodeOp::CryptoDigest {
            dst,
            payload,
            rounds,
        } => {
            write_u8(out, 6);
            write_str(out, dst);
            write_str(out, payload);
            write_u64(out, *rounds);
        }
        BytecodeOp::JsonTransform {
            dst,
            json_payload,
            iterations,
        } => {
            write_u8(out, 7);
            write_str(out, dst);
            write_str(out, json_payload);
            write_u64(out, *iterations);
        }
        BytecodeOp::WinterTcRoute { dst, method, url } => {
            write_u8(out, 8);
            write_str(out, dst);
            write_str(out, method);
            write_str(out, url);
        }
        BytecodeOp::MutateIntrinsic { target } => {
            write_u8(out, 9);
            write_str(out, target);
        }
        BytecodeOp::VfsRead { dst, path } => {
            write_u8(out, 10);
            write_str(out, dst);
            write_str(out, path);
        }
        BytecodeOp::NetFetch { dst, host, path } => {
            write_u8(out, 11);
            write_str(out, dst);
            write_str(out, host);
            write_str(out, path);
        }
        BytecodeOp::EnvGet { dst, key } => {
            write_u8(out, 12);
            write_str(out, dst);
            write_str(out, key);
        }
        BytecodeOp::ConsoleLog { expr } => {
            write_u8(out, 13);
            write_str(out, expr);
        }
        BytecodeOp::ReturnVar { var } => {
            write_u8(out, 14);
            write_str(out, var);
        }
        BytecodeOp::MonomorphicForLoop {
            acc_var,
            idx_var,
            start,
            limit,
            scale,
        } => {
            write_u8(out, 15);
            write_str(out, acc_var);
            write_str(out, idx_var);
            write_i64(out, *start);
            write_i64(out, *limit);
            write_f64(out, *scale);
        }
        BytecodeOp::EvalStmt(stmt) => {
            write_u8(out, 16);
            encode_stmt(out, stmt);
        }
        BytecodeOp::BoaFullEsVm { js_code } => {
            write_u8(out, 17);
            write_str(out, js_code);
        }
        BytecodeOp::AddAssignVar { dst, src } => {
            write_u8(out, 18);
            write_str(out, dst);
            write_str(out, src);
        }
        BytecodeOp::ConsoleLogVar { var } => {
            write_u8(out, 19);
            write_str(out, var);
        }
    }
}

fn encode_stmt_slice(out: &mut Vec<u8>, stmts: &[Stmt]) {
    write_u32(out, stmts.len() as u32);
    for s in stmts {
        encode_stmt(out, s);
    }
}

fn encode_stmt(out: &mut Vec<u8>, stmt: &Stmt) {
    match stmt {
        Stmt::VarDecl {
            name,
            init,
            is_export,
        } => {
            write_u8(out, 0);
            write_str(out, name);
            write_bool(out, *is_export);
            match init {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
        }
        Stmt::ImportCas {
            bindings,
            specifier,
        } => {
            write_u8(out, 1);
            write_str_slice(out, bindings);
            write_str(out, specifier);
        }
        Stmt::FuncDecl { name, params, body } => {
            write_u8(out, 2);
            write_str(out, name);
            write_str_slice(out, params);
            encode_stmt_slice(out, body);
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            write_u8(out, 3);
            encode_expr(out, cond);
            encode_stmt_slice(out, then_branch);
            encode_stmt_slice(out, else_branch);
        }
        Stmt::While { cond, body } => {
            write_u8(out, 4);
            encode_expr(out, cond);
            encode_stmt_slice(out, body);
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            write_u8(out, 5);
            match init {
                Some(s) => {
                    write_u8(out, 1);
                    encode_stmt(out, s);
                }
                None => write_u8(out, 0),
            }
            match cond {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
            match step {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
            encode_stmt_slice(out, body);
        }
        Stmt::Return(opt) => {
            write_u8(out, 6);
            match opt {
                Some(e) => {
                    write_u8(out, 1);
                    encode_expr(out, e);
                }
                None => write_u8(out, 0),
            }
        }
        Stmt::Break => write_u8(out, 7),
        Stmt::Continue => write_u8(out, 8),
        Stmt::Block(stmts) => {
            write_u8(out, 9);
            encode_stmt_slice(out, stmts);
        }
        Stmt::ExprStmt(expr) => {
            write_u8(out, 10);
            encode_expr(out, expr);
        }
    }
}

#[inline]
fn bin_op_to_u8(op: BinOp) -> u8 {
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
fn u8_to_bin_op(b: u8) -> Option<BinOp> {
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
fn unary_op_to_u8(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Neg => 0,
        UnaryOp::Pos => 1,
        UnaryOp::Not => 2,
        UnaryOp::BitNot => 3,
        UnaryOp::TypeOf => 4,
    }
}

#[inline]
fn u8_to_unary_op(b: u8) -> Option<UnaryOp> {
    Some(match b {
        0 => UnaryOp::Neg,
        1 => UnaryOp::Pos,
        2 => UnaryOp::Not,
        3 => UnaryOp::BitNot,
        4 => UnaryOp::TypeOf,
        _ => return None,
    })
}

fn encode_js_value(out: &mut Vec<u8>, val: &JsValue) {
    match val {
        JsValue::Undefined => write_u8(out, 0),
        JsValue::Null => write_u8(out, 1),
        JsValue::Bool(b) => {
            write_u8(out, 2);
            write_bool(out, *b);
        }
        JsValue::Number(n) => {
            write_u8(out, 3);
            write_f64(out, *n);
        }
        JsValue::Str(s) => {
            write_u8(out, 4);
            write_str(out, s);
        }
        JsValue::Array(items) => {
            write_u8(out, 5);
            write_u32(out, items.len() as u32);
            for item in items.iter() {
                encode_js_value(out, item);
            }
        }
        JsValue::Object(map) => {
            write_u8(out, 6);
            write_u32(out, map.len() as u32);
            if map.len() <= 1 {
                for (k, v) in map.iter() {
                    write_str(out, k);
                    encode_js_value(out, v);
                }
            } else {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_unstable();
                for k in keys {
                    write_str(out, k);
                    encode_js_value(out, &map[k]);
                }
            }
        }
        JsValue::Function(func) => {
            write_u8(out, 7);
            write_str(out, &func.name);
            write_str_slice(out, &func.params);
            encode_stmt_slice(out, &func.body);
        }
    }
}

fn encode_expr(out: &mut Vec<u8>, expr: &Expr) {
    match expr {
        Expr::Literal(val) => {
            write_u8(out, 0);
            encode_js_value(out, val);
        }
        Expr::Ident(name) => {
            write_u8(out, 1);
            write_str(out, name);
        }
        Expr::Binary { op, lhs, rhs } => {
            write_u8(out, 2);
            write_u8(out, bin_op_to_u8(*op));
            encode_expr(out, lhs);
            encode_expr(out, rhs);
        }
        Expr::Unary { op, expr: inner } => {
            write_u8(out, 3);
            write_u8(out, unary_op_to_u8(*op));
            encode_expr(out, inner);
        }
        Expr::Assign { name, op, value } => {
            write_u8(out, 4);
            write_str(out, name);
            match op {
                Some(bop) => {
                    write_u8(out, 1);
                    write_u8(out, bin_op_to_u8(*bop));
                }
                None => write_u8(out, 0),
            }
            encode_expr(out, value);
        }
        Expr::PropAssign {
            target_chain,
            prop,
            value,
        } => {
            write_u8(out, 5);
            write_str(out, target_chain);
            write_str(out, prop);
            encode_expr(out, value);
        }
        Expr::IndexAssign {
            target,
            index,
            value,
        } => {
            write_u8(out, 6);
            write_str(out, target);
            encode_expr(out, index);
            encode_expr(out, value);
        }
        Expr::Update {
            name,
            increment,
            prefix,
        } => {
            write_u8(out, 7);
            write_str(out, name);
            write_bool(out, *increment);
            write_bool(out, *prefix);
        }
        Expr::Ternary {
            cond,
            then_expr,
            else_expr,
        } => {
            write_u8(out, 8);
            encode_expr(out, cond);
            encode_expr(out, then_expr);
            encode_expr(out, else_expr);
        }
        Expr::Call { callee, args } => {
            write_u8(out, 9);
            encode_expr(out, callee);
            write_u32(out, args.len() as u32);
            for arg in args {
                encode_expr(out, arg);
            }
        }
        Expr::Member { object, property } => {
            write_u8(out, 10);
            encode_expr(out, object);
            write_str(out, property);
        }
        Expr::Index { object, index } => {
            write_u8(out, 11);
            encode_expr(out, object);
            encode_expr(out, index);
        }
        Expr::ArrayLit(items) => {
            write_u8(out, 12);
            write_u32(out, items.len() as u32);
            for item in items {
                encode_expr(out, item);
            }
        }
        Expr::ObjectLit(entries) => {
            write_u8(out, 13);
            write_u32(out, entries.len() as u32);
            for (k, v) in entries {
                write_str(out, k);
                encode_expr(out, v);
            }
        }
        Expr::ArrowFunc { params, body } => {
            write_u8(out, 14);
            write_str_slice(out, params);
            encode_stmt_slice(out, body);
        }
    }
}

pub(crate) struct SnpCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SnpCursor<'a> {
    #[inline]
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    #[inline]
    pub(crate) fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    #[inline]
    fn read_bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    #[inline]
    pub(crate) fn read_u8(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    #[inline]
    fn read_bool(&mut self) -> Option<bool> {
        match self.read_u8()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    #[inline]
    fn read_u32(&mut self) -> Option<u32> {
        let raw: [u8; 4] = self.read_bytes(4)?.try_into().ok()?;
        Some(u32::from_le_bytes(raw))
    }

    #[inline]
    pub(crate) fn read_u32_len(&mut self) -> Option<usize> {
        let count = usize::try_from(self.read_u32()?).ok()?;
        if count > self.remaining() {
            return None;
        }
        Some(count)
    }

    #[inline]
    pub(crate) fn read_u64(&mut self) -> Option<u64> {
        let raw: [u8; 8] = self.read_bytes(8)?.try_into().ok()?;
        Some(u64::from_le_bytes(raw))
    }

    #[inline]
    pub(crate) fn read_i64(&mut self) -> Option<i64> {
        let raw: [u8; 8] = self.read_bytes(8)?.try_into().ok()?;
        Some(i64::from_le_bytes(raw))
    }

    #[inline]
    pub(crate) fn read_f64(&mut self) -> Option<f64> {
        let raw: [u8; 8] = self.read_bytes(8)?.try_into().ok()?;
        Some(f64::from_le_bytes(raw))
    }

    #[inline]
    pub(crate) fn read_str(&mut self) -> Option<&'a str> {
        let len = self.read_u32_len()?;
        let bytes = self.read_bytes(len)?;
        if bytes.is_ascii() {
            // SAFETY: `bytes.is_ascii()` guarantees every byte is `< 0x80`, which is valid UTF-8.
            return Some(unsafe { std::str::from_utf8_unchecked(bytes) });
        }
        simdutf8::basic::from_utf8(bytes).ok()
    }

    #[inline]
    fn read_string(&mut self) -> Option<String> {
        self.read_str().map(str::to_owned)
    }

    fn read_string_vec(&mut self) -> Option<Vec<String>> {
        let count = self.read_u32_len()?;
        let mut out = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
        for _ in 0..count {
            out.push(self.read_string()?);
        }
        Some(out)
    }

    fn decode_op(&mut self, depth: u16) -> Option<BytecodeOp> {
        if depth > MAX_CODEC_DEPTH {
            return None;
        }
        match self.read_u8()? {
            0 => Some(BytecodeOp::ConstNum {
                dst: self.read_string()?,
                val: self.read_f64()?,
            }),
            1 => Some(BytecodeOp::ConstStr {
                dst: self.read_string()?,
                val: self.read_string()?,
            }),
            2 => Some(BytecodeOp::ImportCas {
                dst: self.read_string()?,
                specifier: self.read_string()?,
            }),
            3 => Some(BytecodeOp::AddNum {
                dst: self.read_string()?,
                lhs: self.read_string()?,
                rhs: self.read_string()?,
            }),
            4 => Some(BytecodeOp::NumericLoopKernel {
                dst: self.read_string()?,
                iterations: self.read_u64()?,
            }),
            5 => Some(BytecodeOp::FibKernel {
                dst: self.read_string()?,
                n: self.read_u64()?,
            }),
            6 => Some(BytecodeOp::CryptoDigest {
                dst: self.read_string()?,
                payload: self.read_string()?,
                rounds: self.read_u64()?,
            }),
            7 => Some(BytecodeOp::JsonTransform {
                dst: self.read_string()?,
                json_payload: self.read_string()?,
                iterations: self.read_u64()?,
            }),
            8 => Some(BytecodeOp::WinterTcRoute {
                dst: self.read_string()?,
                method: self.read_string()?,
                url: self.read_string()?,
            }),
            9 => Some(BytecodeOp::MutateIntrinsic {
                target: self.read_string()?,
            }),
            10 => Some(BytecodeOp::VfsRead {
                dst: self.read_string()?,
                path: self.read_string()?,
            }),
            11 => Some(BytecodeOp::NetFetch {
                dst: self.read_string()?,
                host: self.read_string()?,
                path: self.read_string()?,
            }),
            12 => Some(BytecodeOp::EnvGet {
                dst: self.read_string()?,
                key: self.read_string()?,
            }),
            13 => Some(BytecodeOp::ConsoleLog {
                expr: self.read_string()?,
            }),
            14 => Some(BytecodeOp::ReturnVar {
                var: self.read_string()?,
            }),
            15 => Some(BytecodeOp::MonomorphicForLoop {
                acc_var: self.read_string()?,
                idx_var: self.read_string()?,
                start: self.read_i64()?,
                limit: self.read_i64()?,
                scale: self.read_f64()?,
            }),
            16 => Some(BytecodeOp::EvalStmt(self.decode_stmt(depth + 1)?)),
            17 => Some(BytecodeOp::BoaFullEsVm {
                js_code: self.read_string()?,
            }),
            18 => Some(BytecodeOp::AddAssignVar {
                dst: self.read_string()?,
                src: self.read_string()?,
            }),
            19 => Some(BytecodeOp::ConsoleLogVar {
                var: self.read_string()?,
            }),
            _ => None,
        }
    }

    fn decode_stmt_vec(&mut self, depth: u16) -> Option<Vec<Stmt>> {
        if depth > MAX_CODEC_DEPTH {
            return None;
        }
        let count = self.read_u32_len()?;
        let mut out = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
        for _ in 0..count {
            out.push(self.decode_stmt(depth + 1)?);
        }
        Some(out)
    }

    fn decode_stmt(&mut self, depth: u16) -> Option<Stmt> {
        if depth > MAX_CODEC_DEPTH {
            return None;
        }
        match self.read_u8()? {
            0 => {
                let name = self.read_string()?;
                let is_export = self.read_bool()?;
                let init = match self.read_u8()? {
                    0 => None,
                    1 => Some(self.decode_expr(depth + 1)?),
                    _ => return None,
                };
                Some(Stmt::VarDecl {
                    name,
                    init,
                    is_export,
                })
            }
            1 => Some(Stmt::ImportCas {
                bindings: self.read_string_vec()?,
                specifier: self.read_string()?,
            }),
            2 => Some(Stmt::FuncDecl {
                name: self.read_string()?,
                params: self.read_string_vec()?,
                body: self.decode_stmt_vec(depth + 1)?,
            }),
            3 => Some(Stmt::If {
                cond: self.decode_expr(depth + 1)?,
                then_branch: self.decode_stmt_vec(depth + 1)?,
                else_branch: self.decode_stmt_vec(depth + 1)?,
            }),
            4 => Some(Stmt::While {
                cond: self.decode_expr(depth + 1)?,
                body: self.decode_stmt_vec(depth + 1)?,
            }),
            5 => {
                let init = match self.read_u8()? {
                    0 => None,
                    1 => Some(Box::new(self.decode_stmt(depth + 1)?)),
                    _ => return None,
                };
                let cond = match self.read_u8()? {
                    0 => None,
                    1 => Some(self.decode_expr(depth + 1)?),
                    _ => return None,
                };
                let step = match self.read_u8()? {
                    0 => None,
                    1 => Some(self.decode_expr(depth + 1)?),
                    _ => return None,
                };
                let body = self.decode_stmt_vec(depth + 1)?;
                Some(Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                })
            }
            6 => {
                let opt = match self.read_u8()? {
                    0 => None,
                    1 => Some(self.decode_expr(depth + 1)?),
                    _ => return None,
                };
                Some(Stmt::Return(opt))
            }
            7 => Some(Stmt::Break),
            8 => Some(Stmt::Continue),
            9 => Some(Stmt::Block(self.decode_stmt_vec(depth + 1)?)),
            10 => Some(Stmt::ExprStmt(self.decode_expr(depth + 1)?)),
            _ => None,
        }
    }

    fn decode_js_value(&mut self, depth: u16) -> Option<JsValue> {
        if depth > MAX_CODEC_DEPTH {
            return None;
        }
        match self.read_u8()? {
            0 => Some(JsValue::Undefined),
            1 => Some(JsValue::Null),
            2 => Some(JsValue::Bool(self.read_bool()?)),
            3 => Some(JsValue::Number(self.read_f64()?)),
            4 => Some(JsValue::str(self.read_string()?)),
            5 => {
                let count = self.read_u32_len()?;
                let mut items = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
                for _ in 0..count {
                    items.push(self.decode_js_value(depth + 1)?);
                }
                Some(JsValue::array(items))
            }
            6 => {
                let count = self.read_u32_len()?;
                let mut map = FxHashMap::with_capacity_and_hasher(
                    count.min(MAX_PREALLOC_ITEMS),
                    Default::default(),
                );
                for _ in 0..count {
                    let k = self.read_string()?;
                    let v = self.decode_js_value(depth + 1)?;
                    map.insert(k, v);
                }
                Some(JsValue::Object(Box::new(map)))
            }
            7 => {
                let name = self.read_string()?;
                let params = self.read_string_vec()?;
                let body = self.decode_stmt_vec(depth + 1)?;
                let mut js_fn = JsFunction {
                    name,
                    params,
                    body,
                    compiled_numeric: None,
                };
                js_fn.compiled_numeric = crate::numeric_vm::try_compile_numeric_func(&js_fn);
                Some(JsValue::Function(Box::new(js_fn)))
            }
            _ => None,
        }
    }

    fn decode_expr(&mut self, depth: u16) -> Option<Expr> {
        if depth > MAX_CODEC_DEPTH {
            return None;
        }
        match self.read_u8()? {
            0 => Some(Expr::Literal(self.decode_js_value(depth + 1)?)),
            1 => Some(Expr::Ident(self.read_string()?)),
            2 => Some(Expr::Binary {
                op: u8_to_bin_op(self.read_u8()?)?,
                lhs: Box::new(self.decode_expr(depth + 1)?),
                rhs: Box::new(self.decode_expr(depth + 1)?),
            }),
            3 => Some(Expr::Unary {
                op: u8_to_unary_op(self.read_u8()?)?,
                expr: Box::new(self.decode_expr(depth + 1)?),
            }),
            4 => {
                let name = self.read_string()?;
                let op = match self.read_u8()? {
                    0 => None,
                    1 => Some(u8_to_bin_op(self.read_u8()?)?),
                    _ => return None,
                };
                let value = Box::new(self.decode_expr(depth + 1)?);
                Some(Expr::Assign { name, op, value })
            }
            5 => Some(Expr::PropAssign {
                target_chain: self.read_string()?,
                prop: self.read_string()?,
                value: Box::new(self.decode_expr(depth + 1)?),
            }),
            6 => Some(Expr::IndexAssign {
                target: self.read_string()?,
                index: Box::new(self.decode_expr(depth + 1)?),
                value: Box::new(self.decode_expr(depth + 1)?),
            }),
            7 => Some(Expr::Update {
                name: self.read_string()?,
                increment: self.read_bool()?,
                prefix: self.read_bool()?,
            }),
            8 => Some(Expr::Ternary {
                cond: Box::new(self.decode_expr(depth + 1)?),
                then_expr: Box::new(self.decode_expr(depth + 1)?),
                else_expr: Box::new(self.decode_expr(depth + 1)?),
            }),
            9 => {
                let callee = Box::new(self.decode_expr(depth + 1)?);
                let count = self.read_u32_len()?;
                let mut args = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
                for _ in 0..count {
                    args.push(self.decode_expr(depth + 1)?);
                }
                Some(Expr::Call { callee, args })
            }
            10 => Some(Expr::Member {
                object: Box::new(self.decode_expr(depth + 1)?),
                property: self.read_string()?,
            }),
            11 => Some(Expr::Index {
                object: Box::new(self.decode_expr(depth + 1)?),
                index: Box::new(self.decode_expr(depth + 1)?),
            }),
            12 => {
                let count = self.read_u32_len()?;
                let mut items = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
                for _ in 0..count {
                    items.push(self.decode_expr(depth + 1)?);
                }
                Some(Expr::ArrayLit(items))
            }
            13 => {
                let count = self.read_u32_len()?;
                let mut entries = Vec::with_capacity(count.min(MAX_PREALLOC_ITEMS));
                for _ in 0..count {
                    let k = self.read_string()?;
                    let v = self.decode_expr(depth + 1)?;
                    entries.push((k, v));
                }
                Some(Expr::ObjectLit(entries))
            }
            14 => Some(Expr::ArrowFunc {
                params: self.read_string_vec()?,
                body: self.decode_stmt_vec(depth + 1)?,
            }),
            _ => None,
        }
    }
}
