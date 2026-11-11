# Chapter 08: Embedding Slice in Rust Applications & High-Density MicroVM Swarms

In addition to using the `slice` CLI binary directly, you can embed Slice's 4 modular Rust crates (`slice-engine`, `slice-cas`, `slice-sandbox`, and `slice-io`) as a library inside your own Rust backends, API gateways, database engines, or AI agent hosts.

Because each `SliceIsolate` takes only **`4.0 KB` of memory** and **`~1 microsecond`** to spawn, a single Rust process can host **1,000+ isolated tenant scripts in under `2 milliseconds`** using less than `10 MB` of total RAM (see [`tests/reports/latest.md`](../../tests/reports/latest.md) for live hardware measurements).

This chapter shows you how to add Slice to your Rust `Cargo.toml`, compile and cache TypeScript bundles, execute isolated HTTP handlers and AI sandboxes from Rust, and run 1,000-tenant microVM swarms.

---

## 1. Adding Slice Crates to Your Rust Project (`Cargo.toml`)

Slice is organized into four clean library crates:

| Crate | Purpose for Rust Developers |
| :--- | :--- |
| **[`slice-engine`](../../crates/slice-engine/Cargo.toml)** | `SliceIsolate`, `compile_source_to_bytecode`, `HttpRequestContext`, `SandboxEvalOptions`, `ExecutionMetrics` |
| **[`slice-sandbox`](../../crates/slice-sandbox/Cargo.toml)** | `CapabilityToken`, `SliceManifest`, `SandboxViolation`, `HardwareProtectionDomain`, `parse_dotenv_content` |
| **[`slice-cas`](../../crates/slice-cas/Cargo.toml)** | `CasBuilder`, `MappedCasArchive`, `bundle_entrypoint_with_deps`, `strip_typescript_and_normalize` |
| **[`slice-io`](../../crates/slice-io/Cargo.toml)** | `CompletionRingEngine` (`io_uring` / Windows `IOCP`), `RegisteredBufferPool`, `IoSubmission` |

Add the crates you need to your `Cargo.toml`:
```toml
[dependencies]
slice-engine  = { path = "../slice/crates/slice-engine" }
slice-sandbox = { path = "../slice/crates/slice-sandbox" }
slice-cas     = { path = "../slice/crates/slice-cas" }
slice-io      = { path = "../slice/crates/slice-io" }
anyhow        = "1.0"
```

---

## 2. Recipe 1: Compiling & Running Untrusted TypeScript in a Rust Service

Here is a complete example showing how to pre-warm the thread-local realm pool, compile TypeScript to bytecode once, configure an Object-Capability (`CapabilityToken`) security policy, and execute it inside a fuel- and memory-metered `SliceIsolate`:

```rust
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn main() -> anyhow::Result<()> {
    // 1. Pre-warm the thread-local Boa ES2024 realm pool once per worker thread
    SliceIsolate::prewarm_boa_pool(2);

    // 2. Compile TypeScript source to bytecode once (produces SLICEBC\x01 Precompiled Boa CodeBlock by default, or SLICESNP)
    let ts_source = r#"
        interface Invoice { id: string; subtotal: number; tax: number; }
        const inv: Invoice = { id: "INV-9001", subtotal: 15000, tax: 1200 };
        console.log(JSON.stringify(inv));
        inv.subtotal + inv.tax
    "#;
    let bytecode: Vec<u8> = compile_source_to_bytecode(ts_source);

    // 3. Pre-warm the compiled bundle closure in the worker's realm pool (optional, enables 0-parse calls)
    SliceIsolate::prewarm_boa_bundle(&bytecode);

    // 4. Build an explicit Object-Capability (OCap) token for this tenant
    let caps = CapabilityToken::pure_compute()
        .with_read_prefix("/tmp/tenant-42")
        .with_write_prefix("/tmp/tenant-42")
        .with_net_host("api.stripe.com:443")
        .with_env("STAGE", "production");

    // 5. Spawn a hardware-protected, fuel-metered isolate (500,000 fuel limit, 4 MB memory cage)
    let mut isolate = SliceIsolate::spawn(
        "tenant-42",
        1,              // Hardware protection slot ID
        caps,
        500_000,        // Max CPU instruction fuel
        4 * 1024 * 1024 // Max memory ceiling in bytes (4 MB)
    );

    // 6. Execute bytecode and inspect telemetry
    let metrics = isolate.execute_bytecode(&bytecode, None)?;
    println!("Captured stdout : {:?}", metrics.stdout);
    println!("Return value    : {}", metrics.return_value);
    println!(
        "Executed in     : {:.2} us (Tier: {:?}, Fuel used: {})",
        metrics.total_time_ns as f64 / 1000.0,
        metrics.highest_tier_reached,
        metrics.fuel_consumed
    );
    Ok(())
}
```

---

## 3. Recipe 2: Loading `.env` Files with Automatic Egress DLP in Rust

You can call `isolate.load_dotenv_str(dotenv_content)` directly on any `SliceIsolate` to inject `.env` variables into `process.env` / `env()` while automatically registering sensitive keys (`*_SECRET*`, `*_TOKEN*`, `*_KEY*`, `*_PASSWORD*`, `*_PRIVATE*`) into the isolate's Egress DLP firewall:

```rust
use slice_engine::{compile_source_to_bytecode, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn run_with_dotenv_dlp() {
    let mut isolate = SliceIsolate::spawn(
        "vault-tenant",
        1,
        CapabilityToken::pure_compute(),
        1_000_000,
        4 * 1024 * 1024,
    );

    // Returns (total_keys_loaded, sensitive_secrets_registered_in_dlp)
    let (loaded, dlp_count) = isolate.load_dotenv_str(
        r#"
        SERVICE_NAME=billing-worker
        PUBLIC_KEY_ID="pk_live_12345"
        STRIPE_SECRET_KEY="sk_live_top_secret_998877"
        "#,
    );
    assert_eq!((loaded, dlp_count), (3, 1));

    // If tenant code attempts to print STRIPE_SECRET_KEY (even Base64 encoded), execute_bytecode returns Err(SandboxViolation)!
    let bad_bc = compile_source_to_bytecode(
        r#"console.log(Buffer.from(process.env.STRIPE_SECRET_KEY).toString("base64"));"#,
    );
    let err = isolate.execute_bytecode(&bad_bc, None).unwrap_err();
    println!("Blocked secret exfiltration: {}", err);
}
```

---

## 4. Recipe 3: Dispatching Zero-Copy HTTP Requests from Your Own Rust Web Server

If you have an existing Rust HTTP server (Axum, Actix, Hyper, or custom TCP sockets) and want to execute WinterTC `export default { fetch(req, env, ctx) }` handlers inside `SliceIsolate`, use `HttpRequestContext` and `execute_http_request`:

```rust
use slice_engine::{compile_source_to_bytecode, HttpRequestContext, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn handle_custom_http() -> anyhow::Result<()> {
    SliceIsolate::prewarm_boa_pool(2);

    let handler_ts = r#"
        export default {
            fetch(req: Request): Response {
                const url = new URL(req.url);
                return Response.json({
                    ok: true,
                    path: url.pathname,
                    tenant: req.headers.get("x-slice-tenant")
                });
            }
        };
    "#;
    let bytecode = compile_source_to_bytecode(handler_ts);
    SliceIsolate::prewarm_boa_bundle(&bytecode);

    let mut isolate = SliceIsolate::spawn(
        "tenant-http",
        1,
        CapabilityToken::pure_compute(),
        5_000_000,
        4 * 1024 * 1024,
    );

    // Borrow zero-copy slices directly from your socket buffer
    let headers = [("host", "api.slice.local"), ("x-slice-tenant", "acme-corp")];
    let req_ctx = HttpRequestContext {
        method: "GET",
        path: "/v1/status",
        query: "verbose=1",
        tenant_id: "acme-corp",
        body: "",
        headers: &headers,
    };

    // Reset and execute in microseconds
    isolate.reset_for_request("acme-corp", 1, 5_000_000, 4 * 1024 * 1024);
    let outcome = isolate.execute_http_request(&bytecode, None, &req_ctx)?;

    println!("HTTP {} ({})", outcome.status_code, outcome.content_type);
    println!("Body: {}", outcome.response_body);

    // Run any deferred ctx.waitUntil() callbacks after sending the response to the client
    if outcome.wait_until_count > 0 {
        let _ = isolate.drain_wait_until_tasks();
    }
    Ok(())
}
```

---

## 5. Recipe 4: Using `< 5 us` Checkpoints, `Slice.kv`, and Nested Sandboxes from Rust

```rust
use slice_engine::{SandboxEvalOptions, SliceIsolate};
use slice_sandbox::CapabilityToken;

fn run_ai_agent_host() {
    SliceIsolate::prewarm_boa_pool(2);
    let mut isolate = SliceIsolate::spawn(
        "agent-tenant",
        1,
        CapabilityToken::pure_compute().with_read_prefix("/tmp"),
        2_000_000,
        4 * 1024 * 1024,
    );

    // 1. Write to the tenant's sharded KV store and take a < 5 us snapshot
    isolate.kv_set("step", "baseline", 0).unwrap();
    let snap_id = isolate.checkpoint();

    // 2. Evaluate untrusted LLM TypeScript in an attenuated child sandbox
    let outcome = isolate.eval_sandbox(
        "const answer: number = 21 * 2; answer",
        SandboxEvalOptions {
            fuel: Some(100_000),
            allow_read: vec!["/tmp".to_string()],
            ..Default::default()
        },
    );
    assert!(outcome.ok);
    assert_eq!(outcome.return_value, "42");

    // 3. Mutate KV and rewind to the snapshot in < 5 us
    isolate.kv_set("step", "dirty", 0).unwrap();
    assert!(isolate.restore(snap_id));
    assert_eq!(isolate.kv_get("step").as_deref(), Some("baseline"));
}
```

---

## 6. Running High-Density 1,000-Tenant MicroVM Swarms (`slice microvm-bench`)

To test how your hardware handles spawning and executing 1,000 isolated tenant compartments concurrently, use the built-in `slice microvm-bench` command:

```bash
# Spawn 1,000 isolated tenant compartments and print human-readable report
slice microvm-bench --tenants=1000

# Or emit a JSON report for automated monitoring
slice microvm-bench --tenants=1000 --json
```

On a standard Linux `x86_64` container, `slice microvm-bench --tenants=1000` spawns and executes **all 1,000 isolated tenants in `< 2 ms` total** (`~1 us` P50 cold start, `4.0 KB` per tenant, `< 10 MB` total host RSS). For exact measurements from the latest benchmark run, see [`tests/reports/latest.md`](../../tests/reports/latest.md).
