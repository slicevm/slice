# Contributing to Slice (`Nexus-JS`)

Thank you for your interest in contributing to **Slice (`Nexus-JS`)**! This document outlines the workspace architecture, build and verification workflows, and engineering standards.

---

## 1. Workspace Architecture

Slice is organized as a Cargo workspace with 5 crates:

| Crate | Path | Responsibility |
| :--- | :--- | :--- |
| **`slice-cli`** | [`crates/slice-cli/`](./crates/slice-cli) | Unified CLI binary (`slice run`, `pack`, `build`, `serve`, `test`, `mcp`, `bench`, `eval`, `fmt`, `lint`, `types`, `inspect`, `doctor`). |
| **`slice-engine`** | [`crates/slice-engine/`](./crates/slice-engine) | Hybrid 3-Tier JS/TS/Wasm execution engine (`Tier-0` CAS / `Tier-1` Warm `boa_engine` ES2024+ / `Tier-2` Unboxed Numeric VM & `wasmi`). |
| **`slice-cas`** | [`crates/slice-cas/`](./crates/slice-cas) | Zero-Stat BLAKE3 Content-Addressed Storage (`.cas`), `oxc` TypeScript/JSX compiler, ESM/CJS bundler, and AST tree-shaker. |
| **`slice-sandbox`** | [`crates/slice-sandbox/`](./crates/slice-sandbox) | 5-Ring Zero-Trust security model: OCap capability verification, path traversal cage, Cloud Metadata SSRF shield, and Linux/x86_64 Intel MPK (`pkey_mprotect`). |
| **`slice-io`** | [`crates/slice-io/`](./crates/slice-io) | Cross-platform kernel completion rings (`io_uring` on Linux, `kqueue` on macOS, `IOCP` on Windows). |

### Inside `crates/slice-engine/src/`

`slice-engine` is structured into **7 domain directories**:
- [`bindings/`](./crates/slice-engine/src/bindings): Native Rust `__slice_*` host functions registered into `boa_engine::Context` (OCap-checked FS, Crypto, Fetch, WinterTC, KV/Sandbox, Workers/Sockets, WebSocket, Wasm, Child Process).
- [`intrinsics/`](./crates/slice-engine/src/intrinsics): Hardware-accelerated (`SHA-NI`) and pure-Rust native fast-paths (Hashes/HMAC, AES-CBC/GCM, PBKDF2/HKDF/scrypt, Ed25519/ECDSA, DEFLATE/Zlib/Gzip, WHATWG `URL`/`URLSearchParams`, Base64/Hex, CSPRNG/UUIDv4, SemVer/`ms`/`bytes`).
- [`isolate/`](./crates/slice-engine/src/isolate): `SliceIsolate` lifecycle, 16-way sharded `EDGE_KV_SLAB`, `< 5 us` copy-on-write checkpoints, Egress DLP firewall, and Tier-0/1/2 + HTTP/WS execution entry points.
- [`vm/`](./crates/slice-engine/src/vm): 16-byte `JsValue` layout, AST & register bytecode compiler, unboxed 64-bit numeric VM, and `SLICEBC\x01` `BoaCAS` container.
- [`runtime/`](./crates/slice-engine/src/runtime): Warm `boa_engine` realm pool, virtual-time timer min-heap & stream reactor, Web `Worker` pool, and `wasmi` WebAssembly engine.
- [`net/`](./crates/slice-engine/src/net): Outbound HTTP/1.1 & HTTPS `fetch()`, OCap TCP/TLS/UDP sockets, and RFC 6455 WebSocket codec.
- [`preludes/`](./crates/slice-engine/src/preludes): 19 sharded JS micro-preludes (`00_ultra_core.js` .. `18_intl.js`), Aho-Corasick capability detector, compile-time minifier, and lazy `node:*` module loaders.

---

## 2. Prerequisites & Local Setup

- **Rust Toolchain**: Stable Rust (`1.85+`, `2021` edition) with `cargo` and `clippy`.
- **Build & Test**:
  ```bash
  # Build the workspace
  cargo build --workspace

  # Run the full unit & integration test suite (137+ tests)
  cargo test --workspace

  # Run strict Clippy lints across all targets
  cargo clippy --workspace --all-targets -- -D warnings
  ```
- **Windows Full Verification Script**:
  ```powershell
  .\scripts\verify_all.ps1
  ```

---

## 3. Engineering & Security Guidelines

1. **Zero Ambient Authority (OCap Everywhere)**:
   - Every host binding in `crates/slice-engine/src/bindings/` that touches the filesystem, network, environment, or subprocesses **must** verify the active isolate's `CapabilitySet` before performing any I/O.
   - Child isolates (`Slice.evalSandbox`, `new Worker`) must strictly intersect (`parent_caps.attenuate(&requested_caps)`) with their parent's capabilities.
2. **Respect Sandbox Memory & CPU Budgets**:
   - Host allocations driven by untrusted guest inputs (compression, KDFs, KV storage, VFS writes, WebAssembly linear memory) must check the isolate's active memory budget (`BOA_KV_MEM_BUDGET` / `max_memory_bytes`).
3. **Egress DLP Enforcement**:
   - Any new outbound channel (`stdout`, `stderr`, HTTP response, WebSocket frame, TCP/UDP socket write, `fetch()`) must pass payloads through `check_host_egress_dlp`.
4. **No Emojis & Concise Commits**:
   - Keep code comments, documentation, and commit messages clean and technical (Conventional Commits style: `feat(engine): ...`, `fix(sandbox): ...`, `refactor(cas): ...`).

---

## 4. Submitting a Pull Request

1. Fork the repository and create a topic branch from `main`.
2. Add unit or integration tests covering your change (in `crates/*/tests/`).
3. Ensure `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` pass cleanly.
4. Open a Pull Request filling out the PR template checklist.

---

## License

By contributing to Slice, you agree that your contributions will be dual-licensed under the [MIT License](./LICENSE-MIT) and [Apache License 2.0](./LICENSE-APACHE).
