//! Tier-0 `LLInt` Zero-Copy Binary `SLICESNP\x02` Bytecode Interpreter & CAS Export Binder.
//!
//! Walks flat `SLICESNP\x02` snapshots directly via [`SnpCursor`] without deserializing
//! intermediate `BytecodeModule` or `BytecodeOp` heap structures, and binds pure numeric
//! `SLICESNP` `.cas` exports directly from `mmap` slices with zero `HeapAlloc`s via
//! [`CompactString`] inline keys (`<= 24` bytes).

use crate::isolate::SliceIsolate;
use crate::slicesnp_codec::{
    SLICESNP_FLAG_PURE_STATIC_EXPORTS, SnpCursor, decode_slicesnp_import_cached,
    visit_pure_static_exports,
};
use crate::value_and_ast::{BytecodeOp, ExecutionTier, JsValue, Stmt, TIER1_PROMOTION_THRESHOLD};
use compact_str::CompactString;
use slice_cas::MappedCasArchive;
use slice_sandbox::SandboxViolation;
use std::time::Instant;

impl SliceIsolate {
    #[inline]
    pub(crate) fn set_env_var_str(&mut self, key: &str, val: JsValue) {
        if let Some(slot) = self.env_vars.get_mut(key) {
            *slot = val;
        } else {
            self.env_vars.insert(CompactString::new(key), val);
        }
    }

    #[inline]
    pub(crate) fn exec_console_log_var(&mut self, var: &str) -> JsValue {
        let v = self.env_vars.get(var).unwrap_or(&JsValue::Undefined);
        self.stdout.push(v.to_string());
        JsValue::Undefined
    }

    #[inline]
    pub(crate) fn exec_add_assign_var(
        &mut self,
        dst: &str,
        src: &str,
    ) -> Result<JsValue, SandboxViolation> {
        if let Some(JsValue::Number(b)) = self.env_vars.get(src).cloned()
            && let Some(JsValue::Number(a)) = self.env_vars.get_mut(dst)
        {
            let sum = *a + b;
            *a = sum;
            return Ok(JsValue::Number(sum));
        }
        let lhs = self
            .env_vars
            .get(dst)
            .cloned()
            .unwrap_or(JsValue::Undefined);
        let rhs = self
            .env_vars
            .get(src)
            .cloned()
            .unwrap_or(JsValue::Undefined);
        let next = match (&lhs, &rhs) {
            (JsValue::Str(sa), _) => {
                let s = format!("{}{}", sa, rhs);
                self.compartment.alloc_memory(s.len())?;
                JsValue::str(s)
            }
            (_, JsValue::Str(sb)) => {
                let s = format!("{}{}", lhs, sb);
                self.compartment.alloc_memory(s.len())?;
                JsValue::str(s)
            }
            _ => JsValue::Number(lhs.to_number() + rhs.to_number()),
        };
        self.set_env_var_str(dst, next.clone());
        Ok(next)
    }

    /// Returns `Ok(true)` if `entry` was resolved in Tier-0, or `Ok(false)` if `entry` (or one of
    /// its transitive imports) is a Tier-1 `SLICEBC\x01`/`SLICEBOA` module requiring Tier-1 Boa
    /// promotion of the `.cas` entrypoint.
    pub(crate) fn bind_tier0_cas_entry<'a>(
        &mut self,
        archive: &'a MappedCasArchive,
        entry: &'a slice_cas::CasBlobEntry,
        dst: &str,
        active_hashes: &mut Vec<&'a str>,
    ) -> Result<bool, SandboxViolation> {
        let child_bc = archive.get_bytecode_slice(entry);
        if child_bc.len() >= 8
            && (&child_bc[0..8] == b"SLICEBOA" || &child_bc[0..8] == crate::boacas::BOACAS_MAGIC)
        {
            return Ok(false);
        }
        self.cas_modules_resolved += 1;
        if let Some(exp_iter) = visit_pure_static_exports(child_bc) {
            let (exact_count, _) = exp_iter.size_hint();
            let mut single_val: Option<f64> = None;
            let mut matched_dst = false;
            for (k, v) in exp_iter {
                if k == dst {
                    matched_dst = true;
                }
                if exact_count == 1 {
                    single_val = Some(v);
                }
                self.set_env_var_str(k, JsValue::Number(v));
            }
            if !dst.is_empty()
                && !matched_dst
                && let Some(first_v) = single_val
            {
                self.set_env_var_str(dst, JsValue::Number(first_v));
            }
            return Ok(true);
        }
        let Some(child_mod) = decode_slicesnp_import_cached(child_bc) else {
            return Ok(true);
        };

        if child_mod.ops.len() == child_mod.static_exports.len()
            && child_mod
                .ops
                .iter()
                .all(|op| matches!(op, BytecodeOp::ConstNum { .. }))
        {
            for (k, v) in &child_mod.static_exports {
                self.set_env_var_str(k, JsValue::Number(*v));
            }
            if !dst.is_empty()
                && !child_mod.static_exports.contains_key(dst)
                && child_mod.static_exports.len() == 1
                && let Some((_, first_v)) = child_mod.static_exports.iter().next()
            {
                self.set_env_var_str(dst, JsValue::Number(*first_v));
            }
            return Ok(true);
        }

        if active_hashes.contains(&entry.hash_hex.as_str()) {
            return Ok(true);
        }
        active_hashes.push(entry.hash_hex.as_str());

        let saved_env = std::mem::take(&mut self.env_vars);
        let child_ref = Some(entry.original_specifier.as_str());
        let mut exported_names: Vec<String> = child_mod.static_exports.keys().cloned().collect();
        let mut needs_tier1 = false;

        let eval_res = (|| -> Result<(), SandboxViolation> {
            for op in &child_mod.ops {
                self.compartment.consume_fuel(1)?;
                match op {
                    BytecodeOp::ConstNum { dst: var, val } => {
                        self.set_env_var_str(var, JsValue::Number(*val));
                    }
                    BytecodeOp::ConstStr { dst: var, val } => {
                        self.compartment.alloc_memory(val.len())?;
                        self.set_env_var_str(var, JsValue::str(val.clone()));
                    }
                    BytecodeOp::ImportCas {
                        dst: sub_dst,
                        specifier: sub_spec,
                    } => {
                        if let Some(sub_entry) = archive.resolve_specifier_from(child_ref, sub_spec)
                            && !self.bind_tier0_cas_entry(
                                archive,
                                sub_entry,
                                sub_dst,
                                active_hashes,
                            )?
                        {
                            needs_tier1 = true;
                            return Ok(());
                        }
                    }
                    BytecodeOp::EvalStmt(stmt) => {
                        match stmt {
                            Stmt::VarDecl {
                                name,
                                is_export: true,
                                ..
                            }
                            | Stmt::FuncDecl { name, .. }
                                if !exported_names.contains(name) =>
                            {
                                exported_names.push(name.clone());
                            }
                            _ => {}
                        }
                        let _ = self.eval_stmt(stmt)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        })();

        let child_env = std::mem::replace(&mut self.env_vars, saved_env);
        let _ = active_hashes.pop();
        eval_res?;
        if needs_tier1 {
            return Ok(false);
        }

        for exp_name in &exported_names {
            if let Some(val) = child_env.get(exp_name.as_str()) {
                self.set_env_var_str(exp_name, val.clone());
            }
        }
        if !dst.is_empty()
            && !exported_names.iter().any(|k| k == dst)
            && exported_names.len() == 1
            && let Some(val) = child_env.get(exported_names[0].as_str())
        {
            self.set_env_var_str(dst, val.clone());
        }
        Ok(true)
    }

    pub(crate) fn promote_tier0_cas_to_tier1_boa(
        &mut self,
        archive: &MappedCasArchive,
        start: Instant,
        need_return_str: bool,
    ) -> Result<(u64, u64, String), SandboxViolation> {
        let instantiate_ns = start.elapsed().as_nanos() as u64;
        self.env_vars.clear();
        self.cas_modules_resolved = 0;
        self.current_tier = ExecutionTier::Tier1Baseline;
        let Some(entry_blob) = archive.entrypoint_blob() else {
            let total_ns = start.elapsed().as_nanos() as u64;
            return Ok((instantiate_ns, total_ns, "undefined".to_string()));
        };
        let raw_src = archive.get_source_slice(entry_blob);
        let js_src: std::borrow::Cow<'_, str> = if slice_cas::needs_unbundled_esm_lowering(raw_src)
        {
            std::borrow::Cow::Owned(slice_cas::lower_unbundled_esm(raw_src))
        } else {
            std::borrow::Cow::Borrowed(raw_src)
        };
        let mut cas_guard = crate::boa_realm::ActiveCasArchiveGuard::enter(Some(archive));
        let last_val = self.exec_boa_full_es_vm(&js_src)?;
        self.cas_modules_resolved += cas_guard.finish();
        let has_secrets = !self.egress_audit_secrets.is_empty();
        let has_custom_http_res =
            !need_return_str && crate::boa_realm::BOA_HTTP_RES.with(|res| res.borrow().is_some());
        let ret_str =
            if need_return_str || has_secrets || (!has_custom_http_res && self.stdout.is_empty()) {
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
        let total_ns = start.elapsed().as_nanos() as u64;
        Ok((instantiate_ns, total_ns, ret_str))
    }

    pub(crate) fn execute_slicesnp_v2_flat_llint(
        &mut self,
        bytecode: &[u8],
        cas: Option<&MappedCasArchive>,
        start: Instant,
        need_return_str: bool,
    ) -> Result<(u64, u64, String), SandboxViolation> {
        self.current_tier = ExecutionTier::Tier0LLIntSnapshot;
        self.pending_tier0_throw = None;
        self.last_exit_code = None;
        self.last_process_exited = false;
        let instantiate_ns = start.elapsed().as_nanos() as u64;

        let flags = bytecode[9];
        let mut cur = SnpCursor::new(&bytecode[10..]);
        let Some(export_count) = cur.read_u32_len() else {
            let total_ns = start.elapsed().as_nanos() as u64;
            return Ok((instantiate_ns, total_ns, "undefined".to_string()));
        };
        if export_count > 16 {
            self.env_vars.reserve(export_count.min(4096));
        }
        let mut last_val = JsValue::Undefined;
        for _ in 0..export_count {
            let (Some(k), Some(v)) = (cur.read_str(), cur.read_f64()) else {
                let total_ns = start.elapsed().as_nanos() as u64;
                return Ok((instantiate_ns, total_ns, "undefined".to_string()));
            };
            if (flags & SLICESNP_FLAG_PURE_STATIC_EXPORTS) != 0 {
                self.compartment.consume_fuel(1)?;
                self.set_env_var_str(k, JsValue::Number(v));
                last_val = JsValue::Number(v);
            }
        }

        let Some(op_count) = cur.read_u32_len() else {
            let total_ns = start.elapsed().as_nanos() as u64;
            return Ok((instantiate_ns, total_ns, "undefined".to_string()));
        };
        if op_count > 16 {
            self.env_vars.reserve(op_count.min(4096));
        }
        let entry_referrer = cas
            .and_then(|a| a.entrypoint_blob())
            .map(|e| e.original_specifier.as_str());

        for _ in 0..op_count {
            self.compartment.consume_fuel(1)?;
            let Some(tag) = cur.read_u8() else {
                break;
            };
            match tag {
                0 => {
                    let (Some(dst), Some(val)) = (cur.read_str(), cur.read_f64()) else {
                        break;
                    };
                    self.set_env_var_str(dst, JsValue::Number(val));
                    last_val = JsValue::Number(val);
                }
                1 => {
                    let (Some(dst), Some(val)) = (cur.read_str(), cur.read_str()) else {
                        break;
                    };
                    self.compartment.alloc_memory(val.len())?;
                    let v = JsValue::str(val.to_owned());
                    self.set_env_var_str(dst, v.clone());
                    last_val = v;
                }
                2 => {
                    let (Some(dst), Some(specifier)) = (cur.read_str(), cur.read_str()) else {
                        break;
                    };
                    if let Some(archive) = cas
                        && let Some(entry) =
                            archive.resolve_specifier_from(entry_referrer, specifier)
                    {
                        let mut active_hashes = Vec::new();
                        if !self.bind_tier0_cas_entry(archive, entry, dst, &mut active_hashes)? {
                            return self.promote_tier0_cas_to_tier1_boa(
                                archive,
                                start,
                                need_return_str,
                            );
                        }
                    }
                }
                3 => {
                    let (Some(dst), Some(lhs), Some(rhs)) =
                        (cur.read_str(), cur.read_str(), cur.read_str())
                    else {
                        break;
                    };
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
                    self.set_env_var_str(dst, JsValue::Number(sum));
                    last_val = JsValue::Number(sum);
                }
                4 => {
                    let (Some(dst), Some(iterations)) = (cur.read_str(), cur.read_u64()) else {
                        break;
                    };
                    let val = self.exec_numeric_loop_kernel(iterations)?;
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                5 => {
                    let (Some(dst), Some(n)) = (cur.read_str(), cur.read_u64()) else {
                        break;
                    };
                    let val = self.exec_fib_kernel(n)?;
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                6 => {
                    let (Some(dst), Some(payload), Some(rounds)) =
                        (cur.read_str(), cur.read_str(), cur.read_u64())
                    else {
                        break;
                    };
                    let val = self.exec_crypto_digest(payload, rounds)?;
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                7 => {
                    let (Some(dst), Some(json_payload), Some(iterations)) =
                        (cur.read_str(), cur.read_str(), cur.read_u64())
                    else {
                        break;
                    };
                    self.compartment.consume_fuel(iterations)?;
                    self.current_tier = ExecutionTier::Tier2PeakNative;
                    let parsed: serde_json::Value =
                        serde_json::from_str(json_payload).unwrap_or(serde_json::Value::Null);
                    let out = serde_json::to_string(&parsed).unwrap_or_default();
                    let val = JsValue::str(out);
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                8 => {
                    let (Some(dst), Some(method), Some(url)) =
                        (cur.read_str(), cur.read_str(), cur.read_str())
                    else {
                        break;
                    };
                    let val = self.exec_wintertc_route(method, url)?;
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                9 => {
                    let Some(target) = cur.read_str() else {
                        break;
                    };
                    self.compartment.verify_property_mutation(target)?;
                }
                10 => {
                    let (Some(dst), Some(path)) = (cur.read_str(), cur.read_str()) else {
                        break;
                    };
                    self.compartment.verify_fs_read(path)?;
                    let val = JsValue::str(format!("CAS_VIRTUAL_FILE[{}]", path));
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                11 => {
                    let (Some(dst), Some(host), Some(path)) =
                        (cur.read_str(), cur.read_str(), cur.read_str())
                    else {
                        break;
                    };
                    self.compartment.verify_net_connect(host)?;
                    let val = JsValue::str(format!("HTTP/2 200 OK from {}{}", host, path));
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                12 => {
                    let (Some(dst), Some(key)) = (cur.read_str(), cur.read_str()) else {
                        break;
                    };
                    let s = self.compartment.verify_env_read(key)?.to_string();
                    let val = JsValue::str(s);
                    self.set_env_var_str(dst, val.clone());
                    last_val = val;
                }
                13 => {
                    let Some(expr) = cur.read_str() else {
                        break;
                    };
                    let v = self
                        .env_vars
                        .get(expr)
                        .cloned()
                        .unwrap_or_else(|| JsValue::str(expr.to_owned()));
                    self.stdout.push(v.to_string());
                    last_val = v;
                }
                14 => {
                    let Some(var) = cur.read_str() else {
                        break;
                    };
                    last_val = self
                        .env_vars
                        .get(var)
                        .cloned()
                        .unwrap_or(JsValue::Undefined);
                    break;
                }
                15 => {
                    let (Some(acc_var), Some(idx_var), Some(loop_start), Some(limit), Some(scale)) = (
                        cur.read_str(),
                        cur.read_str(),
                        cur.read_i64(),
                        cur.read_i64(),
                        cur.read_f64(),
                    ) else {
                        break;
                    };
                    let iterations = (limit - loop_start).max(0) as u64;
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
                    if iterations > 0 {
                        let n = iterations as u128;
                        let two_first_last = (loop_start as i128) + (limit as i128) - 1;
                        let arith_sum_i128 = if n.is_multiple_of(2) {
                            ((n / 2) as i128).saturating_mul(two_first_last)
                        } else {
                            (n as i128).saturating_mul(two_first_last / 2)
                        };
                        if (scale - 1.0).abs() < f64::EPSILON {
                            let sum_i: i64 = if n.is_multiple_of(2) {
                                ((n / 2) as u64).wrapping_mul(two_first_last as u64) as i64
                            } else {
                                (n as u64).wrapping_mul((two_first_last / 2) as u64) as i64
                            };
                            acc += sum_i as f64;
                        } else {
                            let exact_int_domain = scale.is_finite()
                                && scale.fract() == 0.0
                                && acc.is_finite()
                                && acc.fract() == 0.0
                                && arith_sum_i128.unsigned_abs() < (1u128 << 53)
                                && (acc.abs() + (arith_sum_i128 as f64 * scale).abs())
                                    < 9_007_199_254_740_992.0;
                            if exact_int_domain {
                                acc += (arith_sum_i128 as f64) * scale;
                            } else {
                                let mut i = loop_start;
                                while i < limit {
                                    acc += (i as f64) * scale;
                                    i += 1;
                                }
                            }
                        }
                    }
                    let res = JsValue::Number(acc);
                    self.set_env_var_str(acc_var, res.clone());
                    self.set_env_var_str(idx_var, JsValue::Number(limit as f64));
                    last_val = res;
                }
                18 => {
                    let (Some(dst), Some(src)) = (cur.read_str(), cur.read_str()) else {
                        break;
                    };
                    last_val = self.exec_add_assign_var(dst, src)?;
                }
                19 => {
                    let Some(var) = cur.read_str() else {
                        break;
                    };
                    last_val = self.exec_console_log_var(var);
                }
                _ => break,
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
}
