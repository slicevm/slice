//! Human-Readable Bytecode Disassembler (`SLICEBC\x01`, `SLICEBOA`, `SLICEERR`, `SLICESNP\x02`).

use super::compiler::SLICEERR_MAGIC;
use crate::slicesnp_codec::decode_slicesnp_module;
use crate::value_and_ast::{BytecodeOp, SNAPSHOT_MAGIC};
use anyhow::{Result, bail};

/// Disassembles a `SLICESNP`, `SLICEBOA`, or `SLICEBC\x01` binary bytecode buffer into human-readable opcodes.
pub fn disassemble_bytecode(bytecode: &[u8]) -> Result<Vec<String>> {
    if let Some((hdr, _pre, _fallback)) = crate::boacas::BoaCasHeader::decode(bytecode) {
        return Ok(vec![format!(
            "0000  BOACAS_PRECOMP  [SLICEBCv1 Approach={} | Flags=0x{:04x} | Shards=0x{:05x} | Precompiled={} B | FallbackJS={} B]",
            hdr.approach_kind,
            hdr.feature_flags,
            hdr.shards_mask,
            hdr.precompiled_len,
            hdr.fallback_js_len
        )]);
    }
    if bytecode.len() >= 8 && &bytecode[0..8] == SLICEERR_MAGIC {
        let msg = std::str::from_utf8(&bytecode[8..]).unwrap_or("Uncaught SyntaxError");
        return Ok(vec![format!("0000  SYNTAX_ERR      [{}]", msg)]);
    }
    if bytecode.len() >= 8 && &bytecode[0..8] == b"SLICEBOA" {
        return Ok(vec![format!(
            "0000  BOA_ES2024_VM   [Zero-Copy Oxc Transpiled JS: {} bytes]",
            bytecode.len() - 8
        )]);
    }
    if bytecode.len() < 8 || &bytecode[0..8] != SNAPSHOT_MAGIC {
        bail!("Invalid SLICESNP bytecode header");
    }
    let module = decode_slicesnp_module(bytecode)?;
    let mut lines = Vec::with_capacity(module.ops.len() + 2);
    for (idx, op) in module.ops.iter().enumerate() {
        let formatted = match op {
            BytecodeOp::ConstNum { dst, val } => {
                format!("{:04}  CONST_NUM       %{} <- {}", idx, dst, val)
            }
            BytecodeOp::ConstStr { dst, val } => {
                format!("{:04}  CONST_STR       %{} <- {:?}", idx, dst, val)
            }
            BytecodeOp::ImportCas { dst, specifier } => {
                format!(
                    "{:04}  IMPORT_CAS      %{} <- cas_resolve({:?})",
                    idx, dst, specifier
                )
            }
            BytecodeOp::AddNum { dst, lhs, rhs } => {
                format!(
                    "{:04}  ADD_F64         %{} <- %{} + %{}",
                    idx, dst, lhs, rhs
                )
            }
            BytecodeOp::NumericLoopKernel { dst, iterations } => {
                format!(
                    "{:04}  LOOP_SIMD_T2    %{} <- sum(0..{})",
                    idx, dst, iterations
                )
            }
            BytecodeOp::FibKernel { dst, n } => {
                format!("{:04}  FIB_NATIVE_T2   %{} <- fib({})", idx, dst, n)
            }
            BytecodeOp::CryptoDigest {
                dst,
                payload,
                rounds,
            } => format!(
                "{:04}  BLAKE3_SIMD_T2  %{} <- digest({:?}, rounds={})",
                idx, dst, payload, rounds
            ),
            BytecodeOp::JsonTransform {
                dst, iterations, ..
            } => {
                format!(
                    "{:04}  JSON_XFORM_T2   %{} <- parse_stringify(iters={})",
                    idx, dst, iterations
                )
            }
            BytecodeOp::WinterTcRoute { dst, method, url } => {
                format!("{:04}  WINTERTC_ROUTE  %{} <- {} {}", idx, dst, method, url)
            }
            BytecodeOp::MutateIntrinsic { target } => {
                format!("{:04}  SES_CHECK_MUT   trap_if_frozen({:?})", idx, target)
            }
            BytecodeOp::VfsRead { dst, path } => {
                format!(
                    "{:04}  OCAP_VFS_READ   %{} <- vfs_read({:?})",
                    idx, dst, path
                )
            }
            BytecodeOp::NetFetch { dst, host, path } => {
                format!(
                    "{:04}  OCAP_NET_FETCH  %{} <- https://{}{}",
                    idx, dst, host, path
                )
            }
            BytecodeOp::EnvGet { dst, key } => {
                format!("{:04}  OCAP_ENV_GET    %{} <- env({:?})", idx, dst, key)
            }
            BytecodeOp::ConsoleLog { expr } => {
                format!("{:04}  STDOUT_EMIT     print(%{})", idx, expr)
            }
            BytecodeOp::ConsoleLogVar { var } => {
                format!("{:04}  STDOUT_VAR      console.log(%{})", idx, var)
            }
            BytecodeOp::AddAssignVar { dst, src } => {
                format!("{:04}  ADD_ASSIGN      %{} += %{}", idx, dst, src)
            }
            BytecodeOp::ReturnVar { var } => {
                format!("{:04}  RET_VAL         return %{}", idx, var)
            }
            BytecodeOp::MonomorphicForLoop {
                acc_var,
                idx_var,
                start,
                limit,
                scale,
            } => format!(
                "{:04}  MONO_FOR_T2     %{} += (%{} in {}..{}) * {}",
                idx, acc_var, idx_var, start, limit, scale
            ),
            BytecodeOp::EvalStmt(stmt) => format!("{:04}  AST_EXEC        {:?}", idx, stmt),
            BytecodeOp::BoaFullEsVm { js_code } => {
                format!(
                    "{:04}  BOA_ES2024_VM   [Oxc Transpiled JS: {} bytes]",
                    idx,
                    js_code.len()
                )
            }
        };
        lines.push(formatted);
    }
    Ok(lines)
}
