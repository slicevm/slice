# Slice (Nexus-JS) — Production Case Studies & Interactive Workbench (`tests/case_studies/`)

This directory contains **5 intensive, multi-file production Case Studies** and an **Interactive Visual Workbench & Terminal Cockpit (`workbench/`)** that showcase and benchmark Slice's core product capabilities:

1. **[`01-tot-autonomous-sre-agent`](./01-tot-autonomous-sre-agent/src/index.ts) — Depth-4 Tree-of-Thought (ToT) Autonomous SRE Remediation Agent:**
   * Evaluates **20 candidate LLM-generated patches** across a **4-level Tree-of-Thought search** over a degraded 4-service cluster (`edge-gateway`, `auth-token-broker`, `ledger-sequencer`, `risk-ml-scorer`).
   * Executes **16 instant `< 5 us` multi-file VFS + `Slice.kv` rollbacks** (`Slice.checkpoint()` / `Slice.restore(id)`) when candidate branches trigger Egress DLP secret exfiltration (Base64/Hex), SES `Object.prototype` pollution, infinite `while(true)` CPU fuel loops, unauthorized `/etc/shadow` & `/etc/passwd` reads, or P99 SLA regressions — committing the 4 optimal patches (`1820 ms -> 248 ms` P99).

2. **[`02-mcp-code-mode-orchestrator`](./02-mcp-code-mode-orchestrator/src/index.ts) — MCP "Code Mode" Orchestrator vs. Chatty Sequential Tool Calling (Guide A Groundwork):**
   * Joins 280 enterprise records across 4 MCP domains (`crm`, `billing`, `support`, `flags`) to identify churn-risk accounts and compute prorated SLA credits.
   * Benchmarks **40 sequential JSON-RPC tool calls (`~39,405` tokens)** against a **single-shot Slice Code Mode sandbox pass (`~631` tokens — `-98.4%` token reduction)** while verifying 100% identical output and trapping 4 adversarial Code Mode probes (`FsReadDenied`, `EGRESS_SECRET_LEAK_BLOCKED`, `IntrinsicMutationBlocked`, `CpuFuelExhausted`).

3. **[`03-multi-tenant-saas-plugin-mesh`](./03-multi-tenant-saas-plugin-mesh/src/index.ts) — Multi-Tenant SaaS Customer Webhook & Plugin Engine (Guide B Groundwork):**
   * Executes **12 customer-authored TypeScript plugins** across **120 live checkout events** with per-tenant `Slice.kv` namespaces and attenuated capabilities.
   * Deterministically isolates **3 noisy-neighbor / malicious tenants** (`tenant-05-noisy-cpu-miner`, `tenant-07-malicious-fs-probe`, `tenant-09-ses-poisoner`) while hot-swapping `tenant-03-berlin-b2b` from `v1.0.0 -> v2.0.0-hotswap` mid-stream with zero dropped events.

4. **[`04-wasm-zero-trust-cdr-pipeline`](./04-wasm-zero-trust-cdr-pipeline/src/index.ts) — Pure-Rust WebAssembly Content Disarm & Reconstruction (CDR) + Zero-Trust PII Vault:**
   * Streams untrusted clinical (`HIPAA`), financial (`PCI`), and infrastructure (`SOC2`) document batches through a custom **WebAssembly binary SimHash kernel** (`cdr-v3.0-zero-trust`), disarms `<script>`/`<iframe>` payloads, vaults **16 PII tokens** (SSNs, credit cards, `sk_live_` keys) with HMAC-SHA256 tags, compresses sanitized archives via RFC 1952 Gzip (`node:zlib`), and verifies Wasm fuel & memory-page cages.

5. **[`05-time-travel-chaos-fuzzer-payments`](./05-time-travel-chaos-fuzzer-payments/src/index.ts) — Deterministic Time-Travel Chaos Fuzzer for Distributed Payment Sagas:**
   * Subjects a 3-phase distributed payment clearing saga (Reserve Escrow -> Clearing House -> Merchant & Fee Settlement) to **48 chaos scenarios** (`CLEARING_HOUSE_TIMEOUT_ABORT`, `MID_FLIGHT_WAL_CORRUPTION_CRASH`, `INSUFFICIENT_LIQUIDITY_OVERDRAFT`, `DUPLICATE_IDEMPOTENCY_REPLAY`).
   * Warps **21 hours** of simulated retry/timeout windows in milliseconds and executes **18 instant `< 5 us` VFS/KV checkpoint rollbacks** on mid-flight crashes, proving **$0.00 double-entry drift** (`$100,000.00` initial supply === `$100,000.00` final supply).

---

## How to Experiment & Visually "Feel" Slice Execution

A cross-platform [`Makefile`](./Makefile) (supporting both **Windows** and **Linux/macOS**) is included in `tests/case_studies` and automatically builds the `slice` release binary and installs `npm` packages on first run:

```bash
cd tests/case_studies

# Show all available targets
make help

# Run the unified 5-in-1 Terminal Visual Cockpit (Plain-English briefings + load telemetry)
make cockpit

# Crank the load 10x across all 5 Case Studies (370 child sandboxes + 340 <5us CoW rollbacks)
make stress

# Run any individual Case Study in Visual ASCII mode (optionally with SCALE=5 or SCALE=10)
make cs01
make cs02 SCALE=5
make cs03
make cs04
make cs05

# Launch the live Interactive Browser Workbench on http://127.0.0.1:3030
make serve

# Run all 5 Time-Travel Test Suites (slice test) + HTTP Workbench smoke test
make test

# Run all 5 Case Studies in canonical JSON benchmark mode
make json

# Build & run the 4-way Docker benchmark (Slice vs Bun vs Deno vs Node.js)
make bench-docker
```

### Option A: Launch the Live Browser Cockpit (`make serve` / `slice serve`)
From `tests/case_studies`, run `make serve` (or run directly via Cargo):

```bash
cd tests/case_studies && make serve
```

Then open **`http://127.0.0.1:3030`** in your browser to:
* Click through **CS-01 to CS-05** and watch live `X-Slice-Isolate-Us`, `X-Slice-Total-Us`, and `X-Slice-Fuel-Used` microVM telemetry headers update on every click.
* Trigger **Live 5-Ring Sandbox Probes** (Clean TS Math, Base64 Secret Leak, SES Prototype Poisoning, Infinite Loop DoS, `/etc/shadow` Escape, and Dirty VFS Rollback) and inspect the real-time trap and `< 5 us` rollback verification.

### Option B: Run the All-In-One Visual Terminal Cockpit (`make cockpit`)
Prints all 5 visual ASCII execution diagrams, Tree-of-Thought branch traces, and security tables directly in your terminal:

```bash
cd tests/case_studies && make cockpit
```

Or run any individual Case Study in **Visual Mode** (`make cs01`..`make cs05`) vs **JSON Benchmark Mode** (`make cs01-json`..`make cs05-json`):

```bash
# Visual ASCII mode in terminal
make -C tests/case_studies cs01

# Deterministic JSON mode (100% byte-for-byte identical across Slice, Bun, Deno, and Node.js)
make -C tests/case_studies cs01-json

# Run the built-in time-travel test suite on all 5 case studies
make -C tests/case_studies test
```

### Option C: Run the Full 4-Runtime Docker Benchmark Suite (`make bench-docker`)

```bash
make -C tests/case_studies bench-docker
```

### Verified Linux Docker (`x86_64`) Case Study Benchmark Results (`SLICEBC\x01` Default, [`bench_latest.md`](../../bench_latest.md))

| Case Study Workload | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Max RSS vs Bun / Node |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **CS 01: `01-tot-autonomous-sre-agent`** (20 ToT Sandboxes + 16 Rollbacks) | **`25.995 ms`** | **`25.069 ms`** | **`14.415 ms`** | `28.510 ms` | `73.275 ms` | `89.907 ms` | **`16.75 MB`** (`31.12 MB` Bun / `83.26 MB` Node) |
| **CS 02: `02-mcp-code-mode-orchestrator`** (`-98.4%` Tokens, 4 Domains) | **`16.875 ms`** (`15.85 ms` Min) | **`16.968 ms`** | **`8.623 ms`** | `16.453 ms` | `43.902 ms` | `66.457 ms` | **`15.94 MB`** (`27.25 MB` Bun / `78.88 MB` Node) |
| **CS 03: `03-multi-tenant-saas-plugin-mesh`** (12 Tenants, 120 Events, Hot-Swap) | **`11.400 ms`** | **`11.720 ms`** | **`5.625 ms`** | `16.897 ms` | `48.875 ms` | `85.829 ms` | **`14.25 MB`** (`26.00 MB` Bun / `85.88 MB` Node) |
| **CS 04: `04-wasm-zero-trust-cdr-pipeline`** (Wasm SimHash CDR + PII Vault) | **`13.108 ms`** | **`13.425 ms`** | **`1.907 ms`** | `19.225 ms` | `45.358 ms` | `67.193 ms` | **`17.00 MB`** (`30.50 MB` Bun / `78.63 MB` Node) |
| **CS 05: `05-time-travel-chaos-fuzzer-payments`** (48 Payment Chaos Sagas) | **`18.270 ms`** | **`18.938 ms`** | **`12.275 ms`** | `19.707 ms` | `63.327 ms` | `82.822 ms` | **`14.38 MB`** (`28.62 MB` Bun / `79.91 MB` Node) |


