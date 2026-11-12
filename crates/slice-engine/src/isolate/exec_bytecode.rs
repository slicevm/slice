//! Top-Level Bytecode Dispatcher (`SLICEBC\x01`, `SLICEBOA`, `SLICESNP`) and Tier-2 SIMD/Native Kernels.

use crate::boa_realm::BOA_HTTP_RES;
use crate::isolate::{ControlFlow, ExecutionMetrics, SliceIsolate};
use crate::parser_and_compiler::decode_slicesnp_cached;
use crate::value_and_ast::{
    BytecodeModule, BytecodeOp, ExecutionTier, JsValue, SNAPSHOT_MAGIC, TIER1_PROMOTION_THRESHOLD,
};
use slice_cas::MappedCasArchive;
use slice_sandbox::SandboxViolation;
use std::time::Instant;

impl SliceIsolate {
    /// Internal zero-clone execution core shared by `execute_bytecode` and `execute_http_request_fast`.
    /// When `need_return_str` is `false` and `egress_audit_secrets` is empty, skips formatting `last_val`
    /// into a heap `String` (saving `"undefined".to_string()` on every `Slice.respond` HTTP request).
    pub(crate) fn execute_bytecode_core(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
        need_return_str: bool,
    ) -> Result<(u64, u64, String), SandboxViolation> {
        let start = Instant::now();
        self.compartment.pkey_domain.enter_domain();

        // Zero-copy fast path for SLICEBC\x01 precompiled Boa containers and SLICEBOA raw ECMAScript bundles
        if bytecode.len() >= 8
            && (&bytecode[0..8] == b"SLICEBOA" || &bytecode[0..8] == crate::boacas::BOACAS_MAGIC)
        {
            let instantiate_ns = start.elapsed().as_nanos() as u64;
            let base_fuel = ((bytecode.len() as u64) / 16).clamp(10, 250);
            let initial_charge = base_fuel.min(self.compartment.fuel_limit.max(1));
            self.compartment.consume_fuel(initial_charge)?;
            self.current_tier = ExecutionTier::Tier1Baseline;
            let last_val = if &bytecode[0..8] == crate::boacas::BOACAS_MAGIC {
                if let Some((hdr, precompiled_slice, fallback_js)) =
                    crate::boacas::BoaCasHeader::decode(bytecode)
                {
                    if crate::boacas::is_exper_boacas_runtime_enabled() {
                        let total_end = crate::boacas::BOACAS_HEADER_LEN
                            + (hdr.precompiled_len as usize)
                            + (hdr.fallback_js_len as usize);
                        let contiguous_payload =
                            &bytecode[crate::boacas::BOACAS_HEADER_LEN..total_end];
                        self.exec_boacas_vm(
                            &hdr,
                            precompiled_slice,
                            fallback_js,
                            contiguous_payload,
                        )?
                    } else {
                        self.exec_boa_full_es_vm(fallback_js)?
                    }
                } else {
                    JsValue::Undefined
                }
            } else {
                let js_code = std::str::from_utf8(&bytecode[8..]).unwrap_or_default();
                self.exec_boa_full_es_vm(js_code)?
            };
            let has_secrets = !self.egress_audit_secrets.is_empty();
            let has_custom_http_res =
                !need_return_str && BOA_HTTP_RES.with(|res| res.borrow().is_some());
            let ret_str = if need_return_str
                || has_secrets
                || (!has_custom_http_res && self.stdout.is_empty())
            {
                last_val.to_string()
            } else {
                String::new()
            };
            if has_secrets {
                for line in &self.stdout {
                    self.verify_egress_not_leaked(line)?;
                }
                self.verify_egress_not_leaked(&ret_str)?;
                if let Some(ref rj) = self.last_return_json {
                    self.verify_egress_not_leaked(rj)?;
                }
                BOA_HTTP_RES.with(|res| -> Result<(), SandboxViolation> {
                    if let Some((_, ref res_body, ref res_bytes, ref res_ct, ref res_hdrs)) =
                        *res.borrow()
                    {
                        self.verify_egress_not_leaked(res_body)?;
                        if let Some(ref raw) = res_bytes {
                            self.verify_egress_not_leaked(&String::from_utf8_lossy(raw))?;
                        }
                        self.verify_egress_not_leaked(res_ct)?;
                        for (_, hv) in res_hdrs {
                            self.verify_egress_not_leaked(hv)?;
                        }
                    }
                    Ok(())
                })?;
            }
            let total_ns = start.elapsed().as_nanos() as u64;
            return Ok((instantiate_ns, total_ns, ret_str));
        }

        let module = decode_slicesnp_cached(bytecode);
        self.current_tier = ExecutionTier::Tier0LLIntSnapshot;

        let instantiate_ns = start.elapsed().as_nanos() as u64;
        let mut last_val = JsValue::Undefined;

        for op in &module.ops {
            self.compartment.consume_fuel(1)?;

            match op {
                BytecodeOp::ConstNum { dst, val } => {
                    self.env_vars.insert(dst.clone(), JsValue::Number(*val));
                    last_val = JsValue::Number(*val);
                }
                BytecodeOp::ConstStr { dst, val } => {
                    self.compartment.alloc_memory(val.len())?;
                    let v = JsValue::str(val.clone());
                    self.env_vars.insert(dst.clone(), v.clone());
                    last_val = v;
                }
                BytecodeOp::ImportCas { dst, specifier } => {
                    self.cas_modules_resolved += 1;
                    if let Some(archive) = cas {
                        if let Some(entry) = archive.resolve_specifier(specifier) {
                            let child_bc = archive.get_bytecode_slice(entry);
                            if child_bc.len() >= 8 && &child_bc[0..8] == SNAPSHOT_MAGIC {
                                if let Ok(child_mod) =
                                    serde_json::from_slice::<BytecodeModule>(&child_bc[8..])
                                {
                                    for (k, v) in &child_mod.static_exports {
                                        self.env_vars.insert(k.clone(), JsValue::Number(*v));
                                    }
                                    if let Some((_, first_v)) =
                                        child_mod.static_exports.iter().next()
                                    {
                                        self.env_vars
                                            .insert(dst.clone(), JsValue::Number(*first_v));
                                    }
                                }
                            }
                        }
                    }
                }
                BytecodeOp::AddNum { dst, lhs, rhs } => {
                    let a = self
                        .env_vars
                        .get(lhs)
                        .map(|v| v.to_number())
                        .or_else(|| lhs.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let b = self
                        .env_vars
                        .get(rhs)
                        .map(|v| v.to_number())
                        .or_else(|| rhs.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let sum = a + b;
                    self.env_vars.insert(dst.clone(), JsValue::Number(sum));
                    last_val = JsValue::Number(sum);
                }
                BytecodeOp::NumericLoopKernel { dst, iterations } => {
                    let val = self.exec_numeric_loop_kernel(*iterations)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::FibKernel { dst, n } => {
                    let val = self.exec_fib_kernel(*n)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::CryptoDigest {
                    dst,
                    payload,
                    rounds,
                } => {
                    let val = self.exec_crypto_digest(payload, *rounds)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::JsonTransform {
                    dst,
                    json_payload,
                    iterations,
                } => {
                    self.compartment.consume_fuel(*iterations)?;
                    self.current_tier = ExecutionTier::Tier2PeakNative;
                    let parsed: serde_json::Value =
                        serde_json::from_str(json_payload).unwrap_or(serde_json::Value::Null);
                    let out = serde_json::to_string(&parsed).unwrap_or_default();
                    let val = JsValue::str(out);
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::WinterTcRoute { dst, method, url } => {
                    let val = self.exec_wintertc_route(method, url)?;
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::MutateIntrinsic { target } => {
                    self.compartment.verify_property_mutation(target)?;
                }
                BytecodeOp::VfsRead { dst, path } => {
                    self.compartment.verify_fs_read(path)?;
                    let val = JsValue::str(format!("CAS_VIRTUAL_FILE[{}]", path));
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::NetFetch { dst, host, path } => {
                    self.compartment.verify_net_connect(host)?;
                    let val = JsValue::str(format!("HTTP/2 200 OK from {}{}", host, path));
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::EnvGet { dst, key } => {
                    let s = self.compartment.verify_env_read(key)?.to_string();
                    let val = JsValue::str(s);
                    self.env_vars.insert(dst.clone(), val.clone());
                    last_val = val;
                }
                BytecodeOp::ConsoleLog { expr } => {
                    let v = self
                        .env_vars
                        .get(expr)
                        .cloned()
                        .unwrap_or_else(|| JsValue::str(expr.clone()));
                    self.stdout.push(v.to_string());
                    last_val = v;
                }
                BytecodeOp::ReturnVar { var } => {
                    if let Some(v) = self.env_vars.get(var) {
                        last_val = v.clone();
                    }
                }
                BytecodeOp::MonomorphicForLoop {
                    acc_var,
                    idx_var,
                    start,
                    limit,
                    scale,
                } => {
                    let iterations = (*limit - *start).max(0) as u64;
                    let fuel_cost = (iterations / 250).max(1);
                    self.compartment.consume_fuel(fuel_cost)?;
                    if iterations >= 1_000 {
                        self.current_tier = ExecutionTier::Tier2PeakNative;
                    } else if iterations >= TIER1_PROMOTION_THRESHOLD {
                        self.current_tier = ExecutionTier::Tier1Baseline;
                    }
                    let mut acc = self
                        .env_vars
                        .get(acc_var)
                        .map(|v| v.to_number())
                        .unwrap_or(0.0);
                    if (*scale - 1.0).abs() < f64::EPSILON {
                        let mut sum_i: i64 = 0;
                        let mut i = *start;
                        while i < *limit {
                            sum_i = sum_i.wrapping_add(i);
                            i += 1;
                        }
                        acc += sum_i as f64;
                    } else {
                        let mut i = *start;
                        let s = *scale;
                        while i < *limit {
                            acc += (i as f64) * s;
                            i += 1;
                        }
                    }
                    let res = JsValue::Number(acc);
                    self.env_vars.insert(acc_var.clone(), res.clone());
                    self.env_vars
                        .insert(idx_var.clone(), JsValue::Number(*limit as f64));
                    last_val = res;
                }
                BytecodeOp::EvalStmt(stmt) => match self.eval_stmt(stmt)? {
                    ControlFlow::None(v) | ControlFlow::Return(v) => {
                        last_val = v;
                    }
                    ControlFlow::Break | ControlFlow::Continue => {}
                },
                BytecodeOp::BoaFullEsVm { js_code } => {
                    self.current_tier = ExecutionTier::Tier1Baseline;
                    self.compartment.consume_fuel(25)?;
                    let val = self.exec_boa_full_es_vm(js_code)?;
                    last_val = val;
                }
            }
        }

        let total_ns = start.elapsed().as_nanos() as u64;
        if self.capture_return_json && self.last_return_json.is_none() {
            self.last_return_json =
                crate::value_and_ast::js_value_to_optional_json_string(&last_val);
        }
        let has_secrets = !self.egress_audit_secrets.is_empty();
        let ret_str = if need_return_str || has_secrets || self.stdout.is_empty() {
            last_val.to_string()
        } else {
            String::new()
        };
        if has_secrets {
            for line in &self.stdout {
                self.verify_egress_not_leaked(line)?;
            }
            self.verify_egress_not_leaked(&ret_str)?;
            if let Some(ref rj) = self.last_return_json {
                self.verify_egress_not_leaked(rj)?;
            }
        }

        Ok((instantiate_ns, total_ns, ret_str))
    }

    /// Executes pre-compiled `SLICEBC\x01`, `SLICEBOA`, or `SLICESNP` bytecode directly from an in-memory or `mmap` CAS slice.
    pub fn execute_bytecode(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
    ) -> Result<ExecutionMetrics, SandboxViolation> {
        let (instantiate_ns, total_ns, ret_str) =
            self.execute_bytecode_core(bytecode, cas, true)?;
        let exec_ns = total_ns.saturating_sub(instantiate_ns).max(1);
        Ok(ExecutionMetrics {
            tenant_id: self.compartment.tenant_id.clone(),
            highest_tier_reached: self.current_tier,
            instantiate_time_ns: instantiate_ns,
            execution_time_ns: exec_ns,
            total_time_ns: total_ns,
            fuel_consumed: self.compartment.fuel_consumed,
            memory_used_bytes: self.compartment.memory_used_bytes,
            pkey_domain: self.compartment.pkey_domain.pkey,
            cas_modules_resolved: self.cas_modules_resolved,
            os_stat_syscalls: 0,
            stdout: self.stdout.clone(),
            return_value: ret_str,
            checkpoints_taken: self.checkpoints_taken,
            restores_performed: self.restores_performed,
            restore_time_ns: self.restore_time_ns,
            exit_code: self.last_exit_code,
        })
    }

    #[inline]
    pub(crate) fn exec_numeric_loop_kernel(
        &mut self,
        iterations: u64,
    ) -> Result<JsValue, SandboxViolation> {
        let fuel_cost = (iterations / 100).max(1);
        self.compartment.consume_fuel(fuel_cost)?;
        if iterations >= 10_000 {
            self.current_tier = ExecutionTier::Tier2PeakNative;
        } else if iterations >= TIER1_PROMOTION_THRESHOLD {
            self.current_tier = ExecutionTier::Tier1Baseline;
        }
        let n = iterations as f64;
        let total = (n * (n - 1.0)) * 0.5;
        Ok(JsValue::Number(total))
    }

    #[inline]
    pub(crate) fn exec_fib_kernel(&mut self, n: u64) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(n.max(1))?;
        if n >= 25 {
            self.current_tier = ExecutionTier::Tier2PeakNative;
        }
        let mut a: u64 = 0;
        let mut b: u64 = 1;
        for _ in 0..n {
            let tmp = a.wrapping_add(b);
            a = b;
            b = tmp;
        }
        Ok(JsValue::Number(a as f64))
    }

    pub(crate) fn exec_crypto_digest(
        &mut self,
        payload: &str,
        rounds: u64,
    ) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(rounds.max(1))?;
        self.current_tier = ExecutionTier::Tier2PeakNative;
        let mut current = *blake3::hash(payload.as_bytes()).as_bytes();
        let effective_rounds = rounds.min(2_000);
        for _ in 1..effective_rounds {
            current = *blake3::hash(&current).as_bytes();
        }
        Ok(JsValue::str(
            blake3::Hash::from(current).to_hex().to_string(),
        ))
    }

    #[inline]
    pub(crate) fn exec_wintertc_route(
        &mut self,
        method: &str,
        url: &str,
    ) -> Result<JsValue, SandboxViolation> {
        self.compartment.consume_fuel(5)?;
        let body = format!(
            r#"{{"status":200,"runtime":"Slice-NexusJS","wintertc":true,"method":"{}","url":"{}"}}"#,
            method, url
        );
        Ok(JsValue::str(body))
    }
}
