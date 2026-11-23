//! Bounds-Checked Binary `SLICESNP\x02` Snapshot Decoder & Zero-Copy `SnpCursor`.

use super::{
    MAX_CODEC_DEPTH, MAX_PREALLOC_ITEMS, SLICESNP_FLAG_FLAT_LLINT,
    SLICESNP_FLAG_PURE_STATIC_EXPORTS, SLICESNP_V2_VERSION, u8_to_bin_op, u8_to_unary_op,
};
use crate::value_and_ast::{
    BytecodeModule, BytecodeOp, Expr, JsFunction, JsValue, SNAPSHOT_MAGIC, Stmt,
};
use anyhow::{Result, bail};
use rustc_hash::FxHashMap;

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
