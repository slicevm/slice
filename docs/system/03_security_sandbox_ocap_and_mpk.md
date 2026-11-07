# `slice-sandbox`: 5-Ring Zero-Trust OCap Containment, Dynamic SES & Hardware MPK

Every `SliceIsolate` executes inside a [`TenantCompartment`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) enforcing five concentric security barriers across Tier-0 `SLICESNP`, Tier-1 `SLICEBOA` (`Boa`), Tier-2 `NumericFunc`, Web Workers, Raw Sockets, and WebAssembly (`wasmi`).

---

## 1. The Five Concentric Containment Rings

1. **Ring 1 — SES Intrinsic Lockdown & Dynamic Prototype Rollback (`IntrinsicRegistry::lockdown` + `__slice_check_ses`):**
   - Freezes `Object.prototype`, `Array.prototype`, `Function.prototype`, `Promise.prototype`, `JSON`, `Math`, and `Reflect`. Any prototype pollution attempt immediately traps with `SandboxViolation::IntrinsicMutationBlocked`.
   - **Avoiding the ES Override Mistake While Blocking Dynamic Prototype Pollution:** Directly calling `Object.freeze(Object.prototype)` in ECMAScript marks inherited properties (`toString`, `valueOf`, `constructor`) as `writable: false`, which causes standard CommonJS modules assigning `Child.prototype.toString = ...` to fail in strict mode (the ECMAScript specification "override mistake"). Slice solves this with a two-layer defense:
     1. Static AST detection of direct mutations to `Object.prototype`, `Array.prototype`, `Function.prototype`, and `String.prototype`.
     2. Post-evaluation runtime verification via cached `WarmBoaRealm::ses_check_fn` (`__slice_check_ses`), which inspects `Object.keys(...)` on the four intrinsic prototypes, immediately deletes any dynamically injected properties (such as `const k = "proto" + "type"; Object[k].polluted = 123`), and traps with `SandboxViolation::IntrinsicMutationBlocked` so the warm realm remains 100% unpolluted for subsequent tenants.
2. **Ring 2 — Attenuated VFS Read & Write Object Capabilities (`FsCap`):**
   - [`verify_fs_read`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) and [`verify_fs_write`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) reject any path containing `..` traversal segments and verify that the target path starts with an explicitly granted `allowed_read_prefixes` or `allowed_write_prefixes` entry from `slice.toml` or `--allow-read`/`--allow-write`.
   - Enforced across **both** Tier-0 `SLICESNP` (`BytecodeOp::VfsRead`) and Tier-1 `SLICEBOA` native `node:fs` host functions (`__slice_fs_read_file_sync`, `__slice_fs_write_file_sync`, `__slice_fs_append_file_sync`, `__slice_fs_copy_file_sync`, `__slice_fs_rename_sync`).
3. **Ring 3 — Outbound Network Allowlisting (`NetCap`) & Pre-Connect DNS Resolved-IP SSRF Firewall:**
   - [`verify_net_connect`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) blocks any `fetch()`, `WebSocket`, or TCP/TLS/UDP socket connection to a `host:port` not explicitly listed in `allowed_net_hosts` (`SandboxViolation::NetAccessDenied`).
   - [`resolve_and_verify_ssrf_socket_addrs`](file:///p:/opensource/slice/crates/slice-engine/src/outbound_http.rs) resolves DNS before `TcpStream::connect_timeout` and inspects every resolved IPv4, IPv6, and IPv4-mapped IPv6 (`::ffff:...`) address against [`is_cloud_metadata_ip`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) (`169.254.169.254`, `fd00:ec2::254`, `100.100.100.200`) and [`is_private_or_loopback_ip`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) (RFC 1918 `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, CGNAT `100.64.0.0/10`, loopback `127.0.0.0/8`, `::1`), defeating DNS-rebinding SSRF attacks.
4. **Ring 4 — Scoped Environment Capabilities (`EnvCap`), `.env` Auto-DLP, & Outbound Egress Secret DLP Firewall:**
   - [`verify_env_read`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) isolates tenant secrets in an explicit per-compartment key-value map (`SandboxViolation::EnvAccessDenied`).
   - When `--env-file=.env` is loaded, [`is_sensitive_env_key_or_val`](file:///p:/opensource/slice/crates/slice-cli/src/config_and_watch.rs) automatically classifies high-entropy tokens and keys matching `SECRET`, `TOKEN`, `KEY`, `PASSWORD`, or `CREDENTIAL` as egress-protected secrets.
   - [`verify_egress_secrets_slice`](file:///p:/opensource/slice/crates/slice-engine/src/state_and_kv.rs) inspects `stdout`, `returnValue`, HTTP response bodies, SSE chunks, outbound `fetch()` URLs/headers/bodies, WebSocket frames, and raw TCP/UDP payloads for raw, standard Base64, and URL-safe Base64 encodings of registered secrets, trapping with `SandboxViolation::EnvAccessDenied { key: "EGRESS_SECRET_LEAK_BLOCKED" }` (`0.00 ns` cost when no secrets are registered).
5. **Ring 5 — Deterministic CPU Fuel & Memory Cage + Linux `WRPKRU` Hardware MPK:**
   - Every operation deducts instruction fuel (`consume_fuel`) and tracks heap bytes (`alloc_memory`), preempting infinite `while (true) {}` loops in `< 10 us` (`SandboxViolation::CpuFuelExhausted`).
   - Before evaluating code in [`SliceIsolate::exec_boa_full_es_vm`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs), Slice configures `realm.ctx.runtime_limits_mut().set_loop_iteration_limit(remaining_fuel)` and `set_recursion_limit(512)`, translating loop or call-stack exhaustion inside `Boa` directly into `SandboxViolation::CpuFuelExhausted`.
   - On Linux `x86_64`, [`HardwareProtectionDomain`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) allocates hardware memory protection keys via `SYS_pkey_alloc` (syscall 330) and switches page permissions in `< 20` CPU cycles using the unprivileged `WRPKRU` instruction.

---

## 2. Untrusted Entrypoint & AI Code-Mode Directive Lockdown (`// @slice-*`)

Implemented in [`ResolvedExecutionConfig::apply_inline_directives`](file:///p:/opensource/slice/crates/slice-cli/src/config_and_watch.rs) and [`SliceManifest`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs):
- **Default Monotonic Attenuation (`trust_directives == false`):** By default, top-of-file `// @slice-*` source directives parsed by [`parse_inline_directives`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) cannot escalate privileges when executing untrusted or AI-generated entrypoints (`slice run agent_script.ts`):
  - `fuel` and `memory_bytes` are clamped via `self.fuel_limit.min(f)` and `self.memory_limit_bytes.min(mb)` (cannot exceed host limits).
  - `allow_read`, `allow_write`, `allow_net`, and `allow_env` are strictly intersected against existing host-granted capabilities in `self.caps` (`slice_sandbox::vfs_prefix_matches` for VFS prefixes, wildcard/exact host matching for `allowed_net_hosts`, and existing key membership for `allowed_env_keys`).
  - `env_file`, `port`, and `workers` inline directives are ignored unless `trust_directives` is `true`, while `audit_egress` remains always-additive.
- **Explicit Host Opt-In (`--trust-directives` / `SLICE_TRUST_DIRECTIVES=1` / `trust_directives = true`):** Only when the host explicitly enables `trust_directives` (via `[capabilities] trust_directives = true` in `slice.toml`, `SLICE_TRUST_DIRECTIVES=1`, or `--trust-directives`) do inline directives apply additively. Passing `--no-directives` or `--untrusted` bypasses `apply_inline_directives` altogether.
- **In-Process `Slice.evalSandbox` & `slice mcp` Immunity:** [`SliceIsolate::eval_sandbox`](file:///p:/opensource/slice/crates/slice-engine/src/isolate.rs) and `slice_eval_ts` never call `apply_inline_directives`, guaranteeing that embedded `// @slice-allow-*` or `// @slice-fuel` comments inside sandboxed code snippets have zero effect on child `CapabilityToken` or resource budgets.

