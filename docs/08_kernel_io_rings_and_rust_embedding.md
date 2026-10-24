# Chapter 08: Kernel Completion Rings (`slice-io`) & Embedding Slice in Rust

This chapter details Slice's direct kernel completion ring engine ([`crates/slice-io/src/lib.rs`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs)) and shows how to embed `slice-engine`, `slice-cas`, `slice-sandbox`, and `slice-io` directly inside your own Rust applications.

---

## 1. Direct Kernel Completion Rings (`slice-io`)

Traditional runtimes (`libuv` in Node.js) rely on readiness polling (`epoll` / `kqueue`) plus a background thread pool for filesystem operations. [`CompletionRingEngine`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs#L112-L280) replaces readiness polling with **Unified Submission/Completion Rings**:

1. **Pre-Registered Fixed Slab Buffers ([`RegisteredBufferPool`](file:///p:/opensource/slice/crates/slice-io/src/lib.rs#L48-L108)):**
   - Pre-allocates `N x 64 KB` (`FIXED_BUFFER_SIZE = 65536`) page-aligned buffers at startup.
   - `acquire_slot()` and `release_slot(slot_idx)` operate in O(1) time using a free-list stack, eliminating per-I/O heap allocations.
2. **Linux `io_uring` Backend (`#[cfg(target_os = "linux")]`):**
   - Initializes `io_uring::IoUring` with `IORING_SETUP_SQPOLL` (`setup_sqpoll(1000)`), allowing the Linux kernel thread to poll submission queue entries (`SQEs`) with **0 system calls on the hot path**.
   - Automatically falls back to standard `io_uring` or user-space completion rings when running inside unprivileged container `seccomp` profiles.
3. **Windows NT `IOCP` Backend (`#[cfg(target_os = "windows")]`):**
   - Binds directly to Windows NT kernel I/O Completion Ports via `CreateIoCompletionPort`, `PostQueuedCompletionStatus`, and `GetQueuedCompletionStatus` (`kernel32.dll` FFI), processing `10,000` completions in `0.02 ms - 5.26 ms`.

---

## 2. Embedding Slice in a Rust Application

The workspace is structured as 4 modular library crates plus the CLI binary crate:
- [`crates/slice-sandbox`](file:///p:/opensource/slice/crates/slice-sandbox/Cargo.toml): `CapabilityToken`, `TenantCompartment`, `HardwareProtectionDomain`, `SandboxViolation`, `SliceManifest`
- [`crates/slice-cas`](file:///p:/opensource/slice/crates/slice-cas/Cargo.toml): `CasBuilder`, `MappedCasArchive`, `strip_typescript_and_normalize`, `bundle_entrypoint_with_deps`
- [`crates/slice-io`](file:///p:/opensource/slice/crates/slice-io/Cargo.toml): `CompletionRingEngine`, `IoSubmission`, `RegisteredBufferPool`
- [`crates/slice-engine`](file:///p:/opensource/slice/crates/slice-engine/Cargo.toml): `SliceIsolate`, `compile_source_to_bytecode`, `HttpRequestContext`, `SandboxEvalOptions`

### Example 1: Compiling and Executing TypeScript in an Isolated Compartment

```rust
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn main() -> anyhow::Result<()> {
    // 1. Pre-warm the thread-local Boa ES2024 realm pool once per worker thread
    SliceIsolate::prewarm_boa_pool(2);

    // 2. Compile TypeScript source to bytecode once (SLICESNP or SLICEBOA)
    let ts_source = r#"
        interface Order { id: string; total: number; }
        const order: Order = { id: "ORD-1", total: 42 * 10 };
        console.log(JSON.stringify(order));
    "#;
    let bytecode = compile_source_to_bytecode(ts_source);

    // 3. Configure fine-grained Object-Capability (OCap) permissions
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/workspace/safe")
        .with_env("STAGE", "production");

    // 4. Spawn a hardware-protected, fuel-metered isolate (500k fuel, 4 MB memory cage)
    let mut isolate = SliceIsolate::spawn("tenant-42", 1, caps, 500_000, 4 * 1024 * 1024);

    // 5. Execute bytecode and inspect telemetry
    let metrics = isolate.execute_bytecode(&bytecode, None)?;
    println!("Output: {:?}", metrics.stdout);
    println!("Total Time: {:.2} us | Fuel Used: {}", metrics.total_time_ns as f64 / 1000.0, metrics.fuel_consumed);
    Ok(())
}
```

### Example 2: Using Checkpoints, Sharded KV, and Nested Sub-Sandboxes from Rust

```rust
use slice_engine::{SandboxEvalOptions, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn run_agent_step() {
    SliceIsolate::prewarm_boa_pool(2);
    let mut isolate = SliceIsolate::spawn(
        "agent-tenant",
        1,
        CapabilityToken::pure_compute().with_read_prefix("/workspace"),
        2_000_000,
        4 * 1024 * 1024,
    );
    isolate.add_egress_audit_secret("sk_live_secret_token");

    // Write to tenant-isolated sharded KV and take a < 5 us checkpoint
    isolate.kv_set("status", "baseline", 0).unwrap();
    let snap_id = isolate.checkpoint();

    // Evaluate untrusted AI-generated TypeScript in an attenuated child sandbox
    let outcome = isolate.eval_sandbox(
        "const x: number = 21 * 2; x",
        SandboxEvalOptions {
            fuel: Some(100_000),
            allow_read: vec!["/workspace".to_string()],
            ..Default::default()
        },
    );
    assert!(outcome.ok);
    assert_eq!(outcome.return_value, "42");

    // Roll back state at any time in < 5 us
    assert!(isolate.restore(snap_id));
}
```

