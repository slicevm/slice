# Slice Docker-Based Real-Life Production Stack Benchmarks

This directory (`docker-tests/`) contains a reproducible Linux x86_64 Docker harness and real-life TypeScript production microservices used to verify **100% output parity** and compare execution latency and peak memory footprint (RSS) across:

1. **Slice (Nexus-JS)**: Hybrid architecture combining:
   - **Oxc (`oxc_parser`, `oxc_semantic`, `oxc_transformer`, `oxc_codegen`)**: Production Rust TypeScript/TSX compiler.
   - **Boa (`boa_engine` v0.22)**: Full ECMAScript specification VM for complex real-world OOP/functional code.
   - **Slice Tier-0/Tier-2 Fast Paths + Zero-Stat CAS**: Sub-3-microsecond snapshot instantiation and zero-`stat()` memory-mapped `.cas` module archives.
2. **Bun (`v1.3.14`)**: JavaScriptCore (JSC) + Zig.
3. **Deno (`v2.8.3`)**: V8 + Rust.
4. **Node.js (`v22.22.3`)**: V8 + `--experimental-strip-types`.

---

## Real-Life Production Workloads Included

1. **[`real-stack-app/src/production_stack.ts`](file:///p:/opensource/slice/docker-tests/real-stack-app/src/production_stack.ts) — E-Commerce & Fintech Checkout Engine**
   - Uses TypeScript interfaces (`OrderItem`, `CustomerProfile`, `CheckoutRequest`, `CheckoutReceipt`), union types, `readonly` properties, classes (`PricingAndRiskEngine`), private fields, functional `.map()` / `.reduce()` / `.some()` array pipelines, regional VAT/sales-tax calculation, coupon & tier discount rules, velocity/geo/GPU fraud risk heuristics, and `JSON.stringify` serialization over 50 multi-tenant checkout transactions.
2. **[`real-stack-app/src/api_gateway_pipeline.ts`](file:///p:/opensource/slice/docker-tests/real-stack-app/src/api_gateway_pipeline.ts) — Edge API Gateway, Auth & Rate-Limiting Pipeline**
   - Uses TypeScript type aliases, interfaces (`HttpRequest`, `HttpResponse`, `TenantQuota`), route matching (`/v1/health`, `/v1/events/ingest`, `/v1/ledger/:id`), Bearer token authentication, per-tenant burst/standard sliding quota enforcement, and telemetry aggregation over 100 simulated API requests.

---

## Running the Real-Stack Docker Benchmark Suite

From the repository root (`p:\opensource\slice`):

```powershell
docker build -f docker-tests/Dockerfile -t slice-real-stack-bench .
docker run --rm --privileged slice-real-stack-bench
```

Or run the script inside the main benchmark image:

```powershell
docker build -t slice-linux-bench .
docker run --rm --privileged slice-linux-bench /app/docker-tests/run_real_stack_benchmarks.sh
```

