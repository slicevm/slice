# Security Policy

Slice (`Nexus-JS`) is designed from the ground up as a **100% pure-Rust, Zero-Trust multi-tenant JavaScript/TypeScript runtime** for untrusted edge workloads and AI Agent "Code Mode" sandboxes. We treat any bypass of our isolation or capability boundaries as a critical security issue.

---

## Supported Versions

Security updates are applied to the latest release series on `main`:

| Version | Supported |
| :--- | :--- |
| `0.10.x` (Current) | Yes |
| `< 0.10.0` | No (Upgrade to `>= 0.10.3`) |

---

## What Qualifies as a Security Vulnerability

We specifically welcome reports affecting any of Slice's **5 Isolation & Defense Rings**:

1. **Ring 1 — Object-Capability (OCap) Boundary Bypass (`slice-sandbox`)**:
   - Reading or writing host paths outside `allow_read` / `allow_write` (e.g., via symlink traversal, path normalization edge cases, or file descriptor confusion).
   - Initiating outbound HTTP/WebSocket/TCP/TLS/UDP connections to hosts or ports not listed in `allow_net`.
   - Reading `process.env` keys not listed in `allow_env` or spawning subprocesses without `allow_exec`.
   - Child isolates (`Slice.evalSandbox(code, childCaps)` or `new Worker(..., { sliceCapabilities })`) escalating privileges beyond their parent isolate's capabilities.
2. **Ring 2 — Dynamic SES (Hardened JavaScript) Escape (`slice-engine`)**:
   - Mutating frozen primordials (`Object.prototype`, `Array.prototype`, `Function.prototype`, etc.) when `--lockdown` is enabled.
   - Accessing raw unattenuated `__slice_*` host bridge functions directly from user/tenant code after prelude bootstrap.
3. **Ring 3 — Deterministic CPU Fuel & Byte-Level Heap Cage Escape**:
   - Bypassing `max_cpu_fuel` via un-metered loops, native intrinsics, or regex/compression amplification (zip bombs, KDF memory exhaustion).
   - Allocating unbounded host memory outside the isolate's `max_memory_bytes` / `BOA_KV_MEM_BUDGET` ceiling.
4. **Ring 4 — Cloud Metadata SSRF Shield & Egress DLP Bypass**:
   - Reaching link-local metadata endpoints (`169.254.169.254`, `100.100.100.200`, `fd00:ec2::254`), loopback, or RFC-1918 private networks when `block_private_ips` is active (including via IPv4-mapped IPv6, octal/hex/decimal IP literals, or URL authority confusion).
   - Exfiltrating registered secrets (`--audit-egress` / `SliceIsolate::add_egress_audit_secret`) through `stdout`, HTTP responses, WebSockets, or raw sockets in raw, hex, Base64, or Base64URL form.
5. **Ring 5 — Cryptographic & Memory Safety Defects**:
   - Weak entropy in CSPRNG/UUID generation, timing side-channels in authentication tags, or memory safety issues in any `unsafe` hardware SIMD/MPK block.

---

## Reporting a Vulnerability

**Please do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please report suspected vulnerabilities privately via **GitHub Private Vulnerability Reporting** on the repository's **Security -> Advisories -> Report a vulnerability** tab:

- **URL**: `https://github.com/slicevm/slice/security/advisories/new`

### What to Include in Your Report
- A minimal JavaScript/TypeScript reproduction script and the CLI flags or `slice.toml` configuration used to run it.
- Which capability boundary or isolation ring was bypassed.
- Expected behavior vs. observed behavior.

We aim to acknowledge reports within **48 hours** and provide a remediation timeline or patch within **7 days** for confirmed vulnerabilities.
