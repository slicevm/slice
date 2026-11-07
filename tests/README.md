# Slice Docker-Based Folder Applications, Frameworks, Case Studies, External NPM Libraries & Disk I/O Benchmark Suite

This directory (`tests/`) contains a reproducible Linux `x86_64` Docker benchmark suite comparing **Slice (Nexus-JS)** against the **absolute latest releases** of **Bun (`v1.4.4`)**, **Deno (`v2.9.8`)**, and **Node.js (`v27.0.0-alpha`)** across **31 real-world workloads**: **24 folder-based production TypeScript applications** ([`tests/apps/`](./apps)), **2 full-stack edge framework suites** ([`tests/frameworks/`](./frameworks/README.md)), and **5 intensive AI Agent / SaaS / Chaos Case Studies** ([`tests/case_studies/`](./case_studies/README.md)).

---

## Latest Runtimes Installed in Docker ([`tests/Dockerfile`](./Dockerfile))

- **Slice (`v0.10.3` — `SLICEBC\x01` Precompiled `CodeBlock` CAS Default):** Hybrid architecture combining **Oxc** (`v0.154.0` production Rust TypeScript compiler + AST ESM-to-CJS module bundler with deferred function module spans (`0x11` tag) + `.wasm` zero-stat module bundler + full `package.json` `"exports"` resolver + static barrel tree-shaker), **Boa (`boa_engine` v0.22 + [`vendor/boa_engine/SLICE_PATCH.md`](../vendor/boa_engine/SLICE_PATCH.md))** full ECMAScript VM with default `SLICEBC\x01` precompiled `Gc<CodeBlock>` + global `Scope` binary payloads (zero runtime `boa_parser` / `ByteCompiler` overhead) and pre-fused `node:*` built-in shards, thread-local pre-warmed context pool (`BOA_CONTEXT_POOL`), pure-Rust **WebAssembly (`wasmi` v2.0.0)** engine with `GLOBAL_WASM_MODULE_CACHE` and 5-Ring CPU fuel & linear memory cages, native Rust fast-path intrinsics (`SemVer`, `SHA-256`/`SHA-384`/`SHA-512`, `HMAC-SHA256`/`HMAC-SHA512`, `Base64URL`, in-memory `VFS` slab cache, WinterTC `URL`/`URLPattern`/`URLSearchParams`/`Headers`/`Request`/`Response`/`ReadableStream`/`TransformStream`/`CompressionStream`/`DecompressionStream`/`TextEncoder`/`TextDecoder`/`crypto.subtle`/`Blob`/`File`/`FormData`/`structuredClone`/`EventTarget`/`AbortSignal`/`WebAssembly`, real `Uint8Array`-backed `Buffer`, `node:*` Core Tier-1 modules including `node:async_hooks` and `node:zlib`, `Slice.streamSSE` / `Response.sse`, OCap-guarded outbound HTTP/1.1 & HTTPS TLS 1.3 `fetch()` with Keep-Alive pooling, hop-by-hop redirect guard, Cloud Metadata SSRF Shield, and Egress DLP, built-in Time-Travel `slice test` runner, OS-native `notify` (`v8.2`) `< 5 ms` `--watch`, and Auto-DLP `.env`), and **Slice Zero-Stat CAS + `--static` BLAKE3 `ETag` Static Server + Tier-0/Tier-2 fast paths**.
- **Node.js (`v27.0.0-alpha` — Absolute Latest Release):** Dynamically resolved at container build time from `https://nodejs.org/dist/index.json` (`.[0].version`).
- **Bun (`v1.4.4` — Absolute Latest Release):** Installed via official `https://bun.sh/install`.
- **Deno (`v2.9.8` — Absolute Latest Release):** Installed via official `https://deno.land/install.sh`.

---

## 24 Folder-Based Production Test Applications (`tests/apps/`)

### 1. [`apps/01-fintech-checkout/`](./apps/01-fintech-checkout/src/index.ts) — Multi-Module TypeScript E-Commerce & SHA-256 Crypto Signing
- **Structure:** [`package.json`](./apps/01-fintech-checkout/package.json), [`src/types.ts`](./apps/01-fintech-checkout/src/types.ts), [`src/audit_crypto.ts`](./apps/01-fintech-checkout/src/audit_crypto.ts), [`src/pricing_engine.ts`](./apps/01-fintech-checkout/src/pricing_engine.ts), [`src/index.ts`](./apps/01-fintech-checkout/src/index.ts).
- **What it tests:** Multi-file TypeScript interfaces, union types, `PricingAndRiskEngine` class, regional VAT/sales tax, tiered + coupon discounts, velocity/geo/GPU fraud risk heuristics, and FIPS-180-4 SHA-256 receipt signing via `node:crypto` across 50 checkout orders.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"01-fintech-checkout","ordersProcessed":50,"approvedCount":50,"aggregateRevenueCents":4932364,"lastSha256Signature":"636588368661f40b"}
  ```

### 2. [`apps/02-npm-ecosystem-suite/`](./apps/02-npm-ecosystem-suite/src/index.ts) — Real External NPM Libraries (`semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`)
- **Structure:** [`package.json`](./apps/02-npm-ecosystem-suite/package.json), [`src/release_planner.ts`](./apps/02-npm-ecosystem-suite/src/release_planner.ts), [`src/config_loader.ts`](./apps/02-npm-ecosystem-suite/src/config_loader.ts), [`src/index.ts`](./apps/02-npm-ecosystem-suite/src/index.ts).
- **What it tests:** Static barrel tree-shaking and execution of real third-party CommonJS/ESM packages from `node_modules/` (`semver.valid`, `semver.coerce`, `semver.satisfies`, `semver.gt`, `ms`, and `ini.parse`) across 20 microservice release manifests and cluster SLA topologies, signed with `node:crypto` SHA-256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"02-npm-ecosystem-suite","libraries":["semver@7.6.3","ms@2.1.3","ini@4.1.3"],"validManifests":20,"rangeSatisfied":5,"upgradesRequired":20,"highestSemver":"5.0.0","totalSlaTimeoutMs":197500,"edgeCacheTtlMs":7200000,"heartbeatInterval":"2 minutes","sha256Digest":"ef97948c06f43da9"}
  ```

### 3. [`apps/03-disk-io-log-pipeline/`](./apps/03-disk-io-log-pipeline/src/index.ts) — High-Throughput Disk I/O, Directory Scan & JSONL Stream Processing
- **Structure:** [`package.json`](./apps/03-disk-io-log-pipeline/package.json), [`src/log_generator.ts`](./apps/03-disk-io-log-pipeline/src/log_generator.ts), [`src/io_aggregator.ts`](./apps/03-disk-io-log-pipeline/src/io_aggregator.ts), [`src/index.ts`](./apps/03-disk-io-log-pipeline/src/index.ts).
- **What it tests:** Intensive filesystem I/O (`node:fs`, `node:path`, `node:os`, `node:crypto`) creating, writing, stating, directory-scanning, reading, parsing, SHA-256 verifying, and unlinking 10 JSONL log shard files (`200` records, `15,796 bytes`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"03-disk-io-log-pipeline","shardsScanned":10,"recordsParsed":200,"totalDiskBytes":15796,"error5xxCount":28,"rateLimit429Count":28,"totalBytesTransferred":851796,"maxLatencyMs":121,"summarySha256":"4cbda5d6e5f1f7ca"}
  ```

### 4. [`apps/04-edge-api-gateway/`](./apps/04-edge-api-gateway/src/index.ts) — Edge API Gateway with External NPM Query Parser (`fast-querystring@1.1.2`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/04-edge-api-gateway/package.json), [`src/auth_middleware.ts`](./apps/04-edge-api-gateway/src/auth_middleware.ts), [`src/rate_limiter.ts`](./apps/04-edge-api-gateway/src/rate_limiter.ts), [`src/router.ts`](./apps/04-edge-api-gateway/src/router.ts), [`src/index.ts`](./apps/04-edge-api-gateway/src/index.ts).
- **What it tests:** URL query parsing with Fastify's `fast-querystring` library, sliding-window tenant quota enforcement with `ms`, Bearer token authentication, SHA-256 audit tagging, and route dispatching over 100 HTTP requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"04-edge-api-gateway","libraries":["fast-querystring@1.1.2","ms@2.1.3"],"totalRequests":100,"status200":75,"status401":25,"status429":0,"status404":0,"totalResponseBytes":6700,"latencyPoints":300,"gatewaySha256":"552ff4506f3699d0"}
  ```

### 5. [`apps/05-jwt-oauth-session-guard/`](./apps/05-jwt-oauth-session-guard/src/index.ts) — Zero-Trust OAuth2 / JWT HMAC-SHA256 & HTTP Cookie Session Guard (`cookie@0.6.0`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/05-jwt-oauth-session-guard/package.json), [`src/jwt_codec.ts`](./apps/05-jwt-oauth-session-guard/src/jwt_codec.ts), [`src/session_manager.ts`](./apps/05-jwt-oauth-session-guard/src/session_manager.ts), [`src/index.ts`](./apps/05-jwt-oauth-session-guard/src/index.ts).
- **What it tests:** RFC 6265 HTTP `Cookie` / `Set-Cookie` parsing and serialization (`cookie.parse`, `cookie.serialize`), Base64URL JWT encoding/decoding (`Buffer.from(..., "base64url")`), RFC 2104 HMAC-SHA256 signature verification (`crypto.createHmac("sha256", secret)`), CSRF double-submit token validation, and RBAC scope authorization across 50 sessions.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"05-jwt-oauth-session-guard","libraries":["cookie@0.6.0","ms@2.1.3"],"sessionsEvaluated":50,"authenticatedCount":45,"csrfVerifiedCount":38,"rbacGrantedCount":12,"deniedCount":38,"totalCookieBytes":5448,"auditSha256":"6c262153f1280c9e"}
  ```

### 6. [`apps/06-telemetry-metrics-pipeline/`](./apps/06-telemetry-metrics-pipeline/src/index.ts) — OpenTelemetry Circular Span Graph, Flatted Serialization & Disk WAL (`bytes@3.1.2`, `flatted@3.3.1`)
- **Structure:** [`package.json`](./apps/06-telemetry-metrics-pipeline/package.json), [`src/span_graph.ts`](./apps/06-telemetry-metrics-pipeline/src/span_graph.ts), [`src/histogram_wal.ts`](./apps/06-telemetry-metrics-pipeline/src/histogram_wal.ts), [`src/index.ts`](./apps/06-telemetry-metrics-pipeline/src/index.ts).
- **What it tests:** Constructing parent<->child circular distributed trace span graphs (`SpanNode`), serializing and deserializing cyclic graphs with `flatted` (`stringify` / `parse`), human-readable memory budget formatting/parsing with `bytes` (`bytes.parse`, `bytes.format`), P50/P95 latency histogram calculation, and WAL persistence (`node:fs` + `node:crypto`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"06-telemetry-metrics-pipeline","libraries":["bytes@3.1.2","flatted@3.3.1"],"tracesProcessed":20,"spansEvaluated":60,"p50DurationUs":949,"p95DurationUs":1072,"totalMemoryFormatted":"4.56MB","walBytesOnDisk":11827,"walSha256":"c93139476b4093bb"}
  ```

### 7. [`apps/07-graphql-ast-validator/`](./apps/07-graphql-ast-validator/src/index.ts) — GraphQL Query AST Depth/Complexity Cost Guard & Deep Resolver (`dlv@1.1.3`, `dset@3.1.3`)
- **Structure:** [`package.json`](./apps/07-graphql-ast-validator/package.json), [`src/graphql_ast.ts`](./apps/07-graphql-ast-validator/src/graphql_ast.ts), [`src/deep_resolver.ts`](./apps/07-graphql-ast-validator/src/deep_resolver.ts), [`src/index.ts`](./apps/07-graphql-ast-validator/src/index.ts).
- **What it tests:** Recursive GraphQL selection-set AST depth and complexity cost analysis (blocking abusive deep queries), paired with deep nested object resolution and projection using `dlv` and `dset` over 60 GraphQL queries.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"07-graphql-ast-validator","libraries":["dlv@1.1.3","dset@3.1.3"],"queriesEvaluated":60,"allowedQueries":48,"blockedDeepQueries":12,"totalComplexityScore":2700,"totalProjectedBytes":11472,"schemaSha256":"48510bb0a32a884d"}
  ```

### 8. [`apps/08-event-driven-state-journal/`](./apps/08-event-driven-state-journal/src/index.ts) — CQRS Event-Sourced Ledger Bus & Append-Only JSONL WAL Replay (`mitt@3.0.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/08-event-driven-state-journal/package.json), [`src/event_ledger.ts`](./apps/08-event-driven-state-journal/src/event_ledger.ts), [`src/journal_replayer.ts`](./apps/08-event-driven-state-journal/src/journal_replayer.ts), [`src/index.ts`](./apps/08-event-driven-state-journal/src/index.ts).
- **What it tests:** Pub/sub event-sourced state machine orchestration via `mitt`, append-only write-ahead log (WAL) streaming (`fs.appendFileSync`), full JSONL state replay, and deterministic state convergence verification across 75 ledger events.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"08-event-driven-state-journal","libraries":["mitt@3.0.1","ms@2.1.3"],"eventsDispatched":75,"appliedCount":41,"rejectedCount":34,"replayedAggregateBalanceCents":95480,"journalDiskBytes":10699,"stateMatchVerified":true,"journalSha256":"4a603ab1b248253a"}
  ```

### 9. [`apps/09-wintertc-web-router/`](./apps/09-wintertc-web-router/src/index.ts) — Standard WinterTC Web API (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`) Edge Router
- **Structure:** [`package.json`](./apps/09-wintertc-web-router/package.json), [`src/types.ts`](./apps/09-wintertc-web-router/src/types.ts), [`src/catalog.ts`](./apps/09-wintertc-web-router/src/catalog.ts), [`src/router.ts`](./apps/09-wintertc-web-router/src/router.ts), [`src/index.ts`](./apps/09-wintertc-web-router/src/index.ts).
- **What it tests:** Standard WinterTC / WHATWG `new Request(url, { method, headers, body })`, `new URL(req.url)`, `url.searchParams.get` / `.getAll` / `.sort`, case-insensitive `Headers`, `Response.json(data, init)`, `Response.redirect(url, 308)`, `await res.json()`, and `node:crypto` HMAC-SHA256 webhook verification across 80 multi-tenant HTTP requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"09-wintertc-web-router","totalRequests":80,"ok200Count":30,"created201Count":16,"redirect308Count":10,"unauthorized401Count":10,"notFound404Count":14,"totalRevenueCents":5255850,"matchedCatalogItems":30,"canonicalQuerySample":"category=compute&minPrice=0&sort=asc&tag=pcie","finalStateHash":"bda3093620e333db"}
  ```

### 10. [`apps/10-fullstack-bff-microservice/`](./apps/10-fullstack-bff-microservice/src/index.ts) — Full-Stack Multi-Tenant BFF Microservice (`cookie@0.6.0`, `dlv@1.1.3`, `ms@2.1.3` + WinterTC Fetch APIs)
- **Structure:** [`package.json`](./apps/10-fullstack-bff-microservice/package.json), [`src/types.ts`](./apps/10-fullstack-bff-microservice/src/types.ts), [`src/cors_etag.ts`](./apps/10-fullstack-bff-microservice/src/cors_etag.ts), [`src/bff_controller.ts`](./apps/10-fullstack-bff-microservice/src/bff_controller.ts), [`src/index.ts`](./apps/10-fullstack-bff-microservice/src/index.ts).
- **What it tests:** Combines standard WinterTC `Request`, `Response`, `Headers`, and `URL` with real external npm packages (`cookie.parse` / `cookie.serialize`, `dlv` deep property resolution, and `ms` cache TTL parsing), `OPTIONS` CORS preflight (`204`), SHA-256 `ETag` conditional `If-None-Match` (`304 Not Modified`), and JSON preference mutations (`201`) across 60 requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"10-fullstack-bff-microservice","totalRequests":60,"corsPreflight204Count":10,"notModified304Count":10,"dashboard200Count":20,"preferences201Count":10,"forbidden403Count":10,"totalWidgetsRendered":80,"totalCacheTtlSec":4200,"finalStateHash":"8b0bf90129dbcc3c"}
  ```

### 11. [`apps/11-static-asset-cdn-edge/`](./apps/11-static-asset-cdn-edge/src/index.ts) — Zero-Stat Static Asset CDN Edge Router (`bytes@3.1.2`, `ms@2.1.3` + `ETag` `304`)
- **Structure:** [`package.json`](./apps/11-static-asset-cdn-edge/package.json), [`src/types.ts`](./apps/11-static-asset-cdn-edge/src/types.ts), [`src/asset_manifest.ts`](./apps/11-static-asset-cdn-edge/src/asset_manifest.ts), [`src/cdn_server.ts`](./apps/11-static-asset-cdn-edge/src/cdn_server.ts), [`src/index.ts`](./apps/11-static-asset-cdn-edge/src/index.ts).
- **What it tests:** Static web asset indexing (`.html`, `.css`, `.js`, `.svg`, `.json`), directory index aliases (`/` -> `/index.html`, `/docs/` -> `/docs/index.html`), SHA-256 `ETag` conditional `If-None-Match` (`304 Not Modified`), `HEAD` vs `GET` negotiation, `Cache-Control` TTL calculation via `ms`, and bandwidth-saved formatting via `bytes` across 80 requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"11-static-asset-cdn-edge","libraries":["bytes@3.1.2","ms@2.1.3"],"totalRequests":80,"hit200Count":41,"notModified304Count":24,"head200Count":7,"notFound404Count":8,"totalBytesServed":4857,"formattedBandwidthSaved":"2.97KB","manifestDigest":"84d4b4108f88f928"}
  ```

### 12. [`apps/12-ai-sse-token-streamer/`](./apps/12-ai-sse-token-streamer/src/index.ts) — AI Agent SSE Token Streamer & Outbound Egress DLP (`mitt@3.0.1`, `ms@2.1.3` + `ReadableStream`)
- **Structure:** [`package.json`](./apps/12-ai-sse-token-streamer/package.json), [`src/types.ts`](./apps/12-ai-sse-token-streamer/src/types.ts), [`src/sse_codec.ts`](./apps/12-ai-sse-token-streamer/src/sse_codec.ts), [`src/agent_streamer.ts`](./apps/12-ai-sse-token-streamer/src/agent_streamer.ts), [`src/index.ts`](./apps/12-ai-sse-token-streamer/src/index.ts).
- **What it tests:** WHATWG Server-Sent Events (`id:`, `event:`, `retry:`, `data:`) token streaming over standard `ReadableStream` (`getReader().read()`), `mitt` pub/sub event bus, and real-time Egress DLP secret redaction (raw + Base64) across 40 AI completion streams (`320` SSE frames).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"12-ai-sse-token-streamer","libraries":["mitt@3.0.1","ms@2.1.3"],"streamsCompleted":40,"totalFramesEmitted":320,"totalTokensStreamed":230,"dlpRedactionsTriggered":10,"totalSseBytes":23410,"heartbeatIntervalMs":15000,"streamAuditHash":"9e53ed87fbc54b01"}
  ```

### 13. [`apps/13-webhook-orchestrator-ssrf/`](./apps/13-webhook-orchestrator-ssrf/src/index.ts) — Outbound `fetch()` Webhook Orchestrator & Cloud Metadata SSRF Firewall (`fast-querystring@1.1.2`, `ms@2.1.3` + Live TCP `fetch`)
- **Structure:** [`package.json`](./apps/13-webhook-orchestrator-ssrf/package.json), [`slice.toml`](./apps/13-webhook-orchestrator-ssrf/slice.toml), [`src/types.ts`](./apps/13-webhook-orchestrator-ssrf/src/types.ts), [`src/ssrf_guard.ts`](./apps/13-webhook-orchestrator-ssrf/src/ssrf_guard.ts), [`src/webhook_dispatcher.ts`](./apps/13-webhook-orchestrator-ssrf/src/webhook_dispatcher.ts), [`src/index.ts`](./apps/13-webhook-orchestrator-ssrf/src/index.ts).
- **What it tests:** Outbound `await fetch(url, { method: "POST", headers, body })` against a live upstream HTTP microservice (`127.0.0.1:18095/upstream/webhook`), `fast-querystring` canonicalization, `ms` retry backoff calculation, `node:crypto` HMAC-SHA256 webhook signature generation, and SSRF defense across 40 webhook jobs (blocking AWS/GCP/Alibaba cloud metadata endpoints, unapproved RFC1918 private CIDRs, and untrusted external domains while delivering 10 live HTTP webhooks).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"13-webhook-orchestrator-ssrf","libraries":["fast-querystring@1.1.2","ms@2.1.3"],"totalJobsProcessed":40,"liveHttpDeliveries":10,"ssrfMetadataBlocked":10,"privateCidrBlocked":10,"untrustedDomainBlocked":10,"retryBackoffMs":80000,"auditSignaturePrefix":"db472120dedc36d5"}
  ```

### 14. [`apps/14-federated-api-mesh-dlp/`](./apps/14-federated-api-mesh-dlp/src/index.ts) — Federated Subgraph API Mesh & Outbound Egress DLP Firewall (`bytes@3.1.2`, `dlv@1.1.3`, `dset@3.1.3` + Live TCP `fetch`)
- **Structure:** [`package.json`](./apps/14-federated-api-mesh-dlp/package.json), [`slice.toml`](./apps/14-federated-api-mesh-dlp/slice.toml), [`src/types.ts`](./apps/14-federated-api-mesh-dlp/src/types.ts), [`src/dlp_firewall.ts`](./apps/14-federated-api-mesh-dlp/src/dlp_firewall.ts), [`src/mesh_federator.ts`](./apps/14-federated-api-mesh-dlp/src/mesh_federator.ts), [`src/index.ts`](./apps/14-federated-api-mesh-dlp/src/index.ts).
- **What it tests:** Outbound `await fetch("http://127.0.0.1:18095/upstream/subgraph/account?...")` subgraph fan-out across 25 federated queries, deep object extraction/stitching with `dlv` and `dset`, payload formatting via `bytes`, and Outbound Egress DLP inspection (catching raw, Base64, and Base64URL secret exfiltration in URLs, headers, and bodies).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"14-federated-api-mesh-dlp","libraries":["bytes@3.1.2","dlv@1.1.3","dset@3.1.3"],"queriesExecuted":25,"subgraphFetchesCompleted":25,"dlpExfiltrationsBlocked":5,"cleanAuditForwards":20,"totalAggregatedBytes":"3.2KB","meshStateHash":"973215696f65c8cd"}
  ```

### 15. [`apps/15-time-travel-test-suite/`](./apps/15-time-travel-test-suite/src/index.ts) — Time-Travel VFS + KV Rollback Verification Suite (`semver@7.6.3`, `ms@2.1.3`, `dlv@1.1.3`, `dset@3.1.3` + `slice test`)
- **Structure:** [`package.json`](./apps/15-time-travel-test-suite/package.json), [`slice.toml`](./apps/15-time-travel-test-suite/slice.toml), [`src/types.ts`](./apps/15-time-travel-test-suite/src/types.ts), [`src/state_machine.ts`](./apps/15-time-travel-test-suite/src/state_machine.ts), [`src/time_travel_harness.ts`](./apps/15-time-travel-test-suite/src/time_travel_harness.ts), [`src/index.ts`](./apps/15-time-travel-test-suite/src/index.ts), [`tests/state_machine.test.ts`](./apps/15-time-travel-test-suite/tests/state_machine.test.ts).
- **What it tests:** Executes 24 state-mutating specifications across 4 suites (`semver.satisfies`, `ms`, `dlv`, `dset`, `node:fs` VFS mutations, and `Slice.kv` balance/version mutations), taking a `< 5 us` `Slice.checkpoint()` before each spec and restoring it via `Slice.restore(snapId)` in `finally` to verify 24/24 zero-leakage VFS and KV state rollbacks. Also includes a native [`tests/state_machine.test.ts`](./apps/15-time-travel-test-suite/tests/state_machine.test.ts) verified via `slice test`.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"15-time-travel-test-suite","libraries":["dlv@1.1.3","dset@3.1.3","ms@2.1.3","semver@7.6.3"],"suitesExecuted":4,"specsExecuted":24,"specsPassed":24,"vfsRollbacksVerified":24,"kvRollbacksVerified":24,"totalRetentionMs":29462400000,"suiteAuditHash":"c624c2019b5c44fd"}
  ```

### 16. [`apps/16-dotenv-dlp-config-vault/`](./apps/16-dotenv-dlp-config-vault/src/index.ts) — Zero-Trust `.env` Config Vault & Auto-DLP Secret Exfiltration Auditor (`ini@4.1.3`, `cookie@0.6.0`, `bytes@3.1.2`)
- **Structure:** [`package.json`](./apps/16-dotenv-dlp-config-vault/package.json), [`slice.toml`](./apps/16-dotenv-dlp-config-vault/slice.toml), [`src/types.ts`](./apps/16-dotenv-dlp-config-vault/src/types.ts), [`src/dotenv_vault.ts`](./apps/16-dotenv-dlp-config-vault/src/dotenv_vault.ts), [`src/auto_dlp_auditor.ts`](./apps/16-dotenv-dlp-config-vault/src/auto_dlp_auditor.ts), [`src/index.ts`](./apps/16-dotenv-dlp-config-vault/src/index.ts).
- **What it tests:** Parses `.env` configuration files (`KEY=VALUE`, `export KEY="VALUE"`, inline comments) alongside `ini` cluster limits, `bytes` payload budgets, and `cookie.serialize` headers, automatically classifying sensitive keys (`STRIPE_SECRET_KEY`, `OPENAI_API_TOKEN`, `DB_MASTER_PASSWORD`, `SIGNING_PRIVATE_KEY`) vs public keys (`PUBLIC_KEY_ID`, `PUBLISHABLE_KEY_WEB`) and auditing 50 outbound telemetry payloads against raw, Base64, and Base64URL secret exfiltration, signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"16-dotenv-dlp-config-vault","libraries":["bytes@3.1.2","cookie@0.6.0","ini@4.1.3"],"dotenvEntriesLoaded":8,"publicKeysAllowed":4,"autoDlpSecretsRegistered":4,"payloadsAudited":50,"cleanPayloadsApproved":40,"plaintextLeaksBlocked":5,"base64LeaksBlocked":5,"totalApprovedCookieBytes":"9.55KB","vaultHmacSignature":"c19618e409a21e7a"}
  ```

### 17. [`apps/17-zero-trust-redirect-keepalive-mesh/`](./apps/17-zero-trust-redirect-keepalive-mesh/src/index.ts) — Zero-Trust Hop-by-Hop Redirect Guard, Keep-Alive Connection Pooling & DNS-Rebinding SSRF Firewall (`bytes@3.1.2`, `cookie@0.6.0`, `fast-querystring@1.1.2`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/17-zero-trust-redirect-keepalive-mesh/package.json), [`slice.toml`](./apps/17-zero-trust-redirect-keepalive-mesh/slice.toml), [`src/types.ts`](./apps/17-zero-trust-redirect-keepalive-mesh/src/types.ts), [`src/redirect_security_guard.ts`](./apps/17-zero-trust-redirect-keepalive-mesh/src/redirect_security_guard.ts), [`src/keepalive_mesh_client.ts`](./apps/17-zero-trust-redirect-keepalive-mesh/src/keepalive_mesh_client.ts), [`src/index.ts`](./apps/17-zero-trust-redirect-keepalive-mesh/src/index.ts).
- **What it tests:** Executes 30 zero-trust API mesh tasks against a live HTTP/1.1 Keep-Alive upstream server (`127.0.0.1:18095`), exercising thread-local Keep-Alive connection reuse (`OUTBOUND_CONN_POOL`), `307 Temporary Redirect` hop-by-hop following (`res.redirected === true`, `res.url` tracking), `302 Found` manual redirect inspection (`redirect: "manual"`), cross-origin `Authorization` / `Cookie` stripping, Cloud Metadata (`169.254.169.254`, `fd00:ec2::254`) & DNS-rebinding private IP (`100.64.0.15`) redirect blocking, and Outbound Egress DLP, signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"17-zero-trust-redirect-keepalive-mesh","libraries":["bytes@3.1.2","cookie@0.6.0","fast-querystring@1.1.2","ms@2.1.3"],"tasksProcessed":30,"keepAliveSubgraphHits":5,"redirectFollow307Completed":5,"redirectManual302Inspected":5,"ssrfRedirectMetadataBlocked":5,"dnsRebindPrivateIpBlocked":5,"dlpExfiltrationBlocked":5,"crossOriginAuthStrippedCount":15,"totalTimeoutBudgetMs":50000,"formattedPayloadVolume":"860B","meshAuditHmac":"90f5a8253a5d07de"}
  ```

### 18. [`apps/18-async-event-loop-fanout-scheduler/`](./apps/18-async-event-loop-fanout-scheduler/src/index.ts) — Deterministic Time-Travel Event Loop, Timer Wheel, Microtask Queue, `AbortSignal`, & Concurrent `Promise.all` Fetch Fan-Out (`bytes@3.1.2`, `flatted@3.3.1`, `mitt@3.0.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/18-async-event-loop-fanout-scheduler/package.json), [`slice.toml`](./apps/18-async-event-loop-fanout-scheduler/slice.toml), [`src/types.ts`](./apps/18-async-event-loop-fanout-scheduler/src/types.ts), [`src/timer_retry_scheduler.ts`](./apps/18-async-event-loop-fanout-scheduler/src/timer_retry_scheduler.ts), [`src/concurrent_fanout_client.ts`](./apps/18-async-event-loop-fanout-scheduler/src/concurrent_fanout_client.ts), [`src/index.ts`](./apps/18-async-event-loop-fanout-scheduler/src/index.ts).
- **What it tests:** Verifies deterministic microtask/macrotask priority ordering (`Promise.then`, `queueMicrotask`, `setTimeout` with extra arguments, `clearTimeout`, `setInterval`, `clearInterval`), exponential backoff retry scheduling with `ms`, 5 rounds of 5-shard concurrent `Promise.all([fetch(...), ...])` fan-out with `flatted` + `bytes` + `mitt` event bus, `AbortController` / `AbortSignal` task cancellation, and sandbox timer-bomb containment (`MAX_ACTIVE_TIMERS`), signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"18-async-event-loop-fanout-scheduler","libraries":["bytes@3.1.2","flatted@3.3.1","mitt@3.0.1","ms@2.1.3"],"scheduleOrder":"promise-micro-1>queue-micro-1>timer-stage-a:p1>micro-inside-timer>heartbeat:1>heartbeat:2>heartbeat:3","intervalTicks":3,"cancelledTimerFired":false,"totalRetryBackoffMs":50,"totalShardsExecuted":25,"totalRecordsProcessed":1250,"totalAbortedTasks":5,"totalRetryAttempts":30,"busEventsDispatched":5,"timerBombContained":true,"formattedWireVolume":"1.92KB","auditDigest":"0b76cd366dc934f2"}
  ```

### 19. [`apps/19-wintertc-webcrypto-streams-edge/`](./apps/19-wintertc-webcrypto-streams-edge/src/index.ts) — WinterTC Minimum Common Web Platform API: WebCrypto (`crypto.subtle` HMAC/SHA/JWK), `TextEncoder`/`TextDecoder`, WhatWG Streams (`TransformStream`/`TextEncoderStream`), `EventTarget`, `AbortSignal.any`, `Blob`/`File`/`FormData`, & `structuredClone` (`bytes@3.1.2`, `cookie@1.0.2`, `flatted@3.3.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](./apps/19-wintertc-webcrypto-streams-edge/package.json), [`slice.toml`](./apps/19-wintertc-webcrypto-streams-edge/slice.toml), [`src/types.ts`](./apps/19-wintertc-webcrypto-streams-edge/src/types.ts), [`src/webcrypto_jwt_verifier.ts`](./apps/19-wintertc-webcrypto-streams-edge/src/webcrypto_jwt_verifier.ts), [`src/streams_and_formdata_pipeline.ts`](./apps/19-wintertc-webcrypto-streams-edge/src/streams_and_formdata_pipeline.ts), [`src/index.ts`](./apps/19-wintertc-webcrypto-streams-edge/src/index.ts).
- **What it tests:** Executes a full WinterTC Edge Gateway & Webhook Verification pipeline using `crypto.subtle` (`importKey`, `exportKey` JWK, `sign`, `verify` HS256 & HS512, `digest` SHA-256 & SHA-512), `TextEncoder` (`encode`, `encodeInto` with multi-byte UTF-8 boundary truncation) & `TextDecoder` (`fatal: true`), `btoa`/`atob`, `EventTarget` (`{ once: true, signal }`), `CustomEvent`, `AbortSignal.any`, `Blob` (`.slice()`, `.text()`), `File`, `FormData`, cyclic `structuredClone` (`Map`, `Set`, `Uint8Array`), and a 5-stage WhatWG Stream pipeline (`ReadableStream` -> `TransformStream` -> `TextEncoderStream` -> `TransformStream` -> `TextDecoderStream` -> `WritableStream`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"19-wintertc-webcrypto-streams-edge","libraries":["bytes@3.1.2","cookie@1.0.2","flatted@3.3.1","ms@2.1.3"],"hs256TokenValid":true,"hs256TamperedRejected":true,"hs512WebhookValid":true,"jwkAlg":"HS256","encodeIntoRead":8,"encodeIntoWritten":10,"transformedPayload":"ROUTE:API  |AUTH:JWT  |STREAM:OK::EOF","totalTransformedBytes":37,"formFieldsCount":3,"uploadedFileName":"bundle-manifest.bin","uploadedFileSize":38,"slicedHeaderText":"SLICE-EDGE","cyclicCloneVerified":true,"dispatchedEvents":["once:init","live:init","live:transform"],"anySignalAbortedBy":"secondary-deadline-exceeded","formattedWireVolume":"75B","auditDigest":"7319225b3dc10146"}
  ```

### 20. [`apps/20-node-compat-and-edge-router/`](./apps/20-node-compat-and-edge-router/src/index.ts) — Real `Uint8Array`-Backed `Buffer` Wire Framing, `node:*` Core Tier-1 (`node:events`, `node:path`, `node:crypto`, `node:assert`, `node:util`), & Oxc AST Bundler (`bytes@3.1.2`, `cookie@1.0.2`, `ms@2.1.3`, `semver@7.6.3`)
- **Structure:** [`package.json`](./apps/20-node-compat-and-edge-router/package.json), [`slice.toml`](./apps/20-node-compat-and-edge-router/slice.toml), [`src/types.ts`](./apps/20-node-compat-and-edge-router/src/types.ts), [`src/binary_wire_codec.ts`](./apps/20-node-compat-and-edge-router/src/binary_wire_codec.ts), [`src/router_and_events_bus.ts`](./apps/20-node-compat-and-edge-router/src/router_and_events_bus.ts), [`src/index.ts`](./apps/20-node-compat-and-edge-router/src/index.ts).
- **What it tests:** Binary wire protocol framing (`writeUInt32BE`, `writeUInt16BE`, `writeBigUInt64BE`, `readUInt32BE`, `readUInt16BE`, `readBigUInt64BE`, `Buffer.concat`, `.subarray()`, `.equals()`), streaming multi-chunk `node:crypto` `createHash("sha512")` & `createHmac("sha256")` returning `Buffer`, `node:events` `EventEmitter` (`prependListener`, `on`, `once`, `emit`), `node:path` POSIX normalization/relative/parse/format, `node:util` (`format`, `promisify`, `types`), `node:assert`, and Oxc AST multi-file ESM-to-CJS bundling.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"20-node-compat-and-edge-router","wireFrame":{"magicHex":"534c4334","version":4,"flags":258,"sequenceId":"9007199254740993","payloadUtf8":"{\"rpc\":\"edge.dispatch\",\"region\":\"iad-01\",\"currency\":\"€\"}","payloadBase64Url":"eyJycGMiOiJlZGdlLmRpc3BhdGNoIiwicmVnaW9uIjoiaWFkLTAxIiwiY3VycmVuY3kiOiLigqwifQ","sha512DigestPrefix":"06c23b3b4132558f8dd7b430","hmacSha256Hex":"fda684d490d2bd86c7023653983dab877506c69edf9bb254675a7305e9d91c6c","isValidBufferInstance":true},"routeSummary":{"normalizedRoute":"/api/v2/ tenants/dispatch/invoke","relativeAssetPath":"edge/assets/bundles/client.v4.mjs","parsedBase":"client.v4.mjs","sessionTenant":"acme-fintech-eu","ttlMs":900000,"maxBodyBytes":65536,"engineSemverSatisfied":true,"formattedAuditLine":"tenant=acme-fintech-eu route=/api/v2/ tenants/dispatch/invoke status=200 asset=client.v4.mjs","eventSequence":["prepend:/api/v2/ tenants/dispatch/invoke","on:/api/v2/ tenants/dispatch/invoke","auth-once:acme-fintech-eu","done:200"]}}
  ```

### 21. [`apps/21-websocket-realtime-and-security-guard/`](./apps/21-websocket-realtime-and-security-guard/src/index.ts) — RFC 6455 `WebSocket` Client, 5-Ring Security Firewall & Extended Node Core (`node:url`, `node:querystring`, `node:string_decoder`, `node:timers`, `node:zlib`)
- **Structure:** [`package.json`](./apps/21-websocket-realtime-and-security-guard/package.json), [`slice.toml`](./apps/21-websocket-realtime-and-security-guard/slice.toml), [`src/types.ts`](./apps/21-websocket-realtime-and-security-guard/src/types.ts), [`src/ws_security_firewall.ts`](./apps/21-websocket-realtime-and-security-guard/src/ws_security_firewall.ts), [`src/ws_realtime_session.ts`](./apps/21-websocket-realtime-and-security-guard/src/ws_realtime_session.ts), [`src/index.ts`](./apps/21-websocket-realtime-and-security-guard/src/index.ts).
- **What it tests:** Live RFC 6455 `WebSocket` handshake, subprotocol negotiation (`slice.realtime.v1`), text/JSON/binary `ArrayBuffer` frame exchanges, `close(1000)` handshake, 5-Ring WebSocket Security Firewall (CSWSH Origin guard, SSRF private IP/metadata block, Egress DLP raw + Base64 redaction, frame/connection quota guard, and `node:zlib` zip-bomb ratio enforcement), plus `node:string_decoder`, `node:querystring`, `node:url`, and `node:timers/promises`.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"21-websocket-realtime-and-security-guard","auditSummary":{"totalCasesEvaluated":11,"ring1OriginBlocked":2,"ring2SsrfBlocked":3,"ring3DlpBlocked":2,"ring4QuotaBlocked":2,"ring5DecompressionBombBlocked":1,"cleanFramesAllowed":1,"gzipRoundtripVerified":true,"auditDigest":"fd708b824a057e7a"},"sessionSummary":{"subprotocolNegotiated":"slice.realtime.v1","textFramesSent":3,"textFramesReceived":3,"binaryBytesEchoed":16,"utf8MultiByteDecoded":"Slice-Edge-€-🚀-Realtime","canonicalQuery":"channel=alpha&cursor=100&mode=realtime&tenant=acme-eu","closeCode":1000,"closeClean":true,"sessionTranscriptDigest":"6e74636db07fd3f8"}}
  ```

### 22. [`apps/22-worker-threads-and-raw-sockets/`](./apps/22-worker-threads-and-raw-sockets/src/index.ts) — Multi-Isolate `Worker` / `node:worker_threads`, `MessageChannel`, Raw TCP/UDP Sockets (`node:net`, `node:dgram`), & 5-Ring Socket/Worker Security Firewall
- **Structure:** [`package.json`](./apps/22-worker-threads-and-raw-sockets/package.json), [`slice.toml`](./apps/22-worker-threads-and-raw-sockets/slice.toml), [`src/types.ts`](./apps/22-worker-threads-and-raw-sockets/src/types.ts), [`src/socket_and_worker_firewall.ts`](./apps/22-worker-threads-and-raw-sockets/src/socket_and_worker_firewall.ts), [`src/worker_and_socket_orchestrator.ts`](./apps/22-worker-threads-and-raw-sockets/src/worker_and_socket_orchestrator.ts), [`src/index.ts`](./apps/22-worker-threads-and-raw-sockets/src/index.ts).
- **What it tests:** Spawns 3 isolated `node:worker_threads` workers with `workerData` and `postMessage`, synchronous `MessageChannel` / `MessagePort` IPC, live `node:net` TCP client-server command multiplexing (`PING`, `PUT`, `STATUS`, `QUIT`), live `node:dgram` UDP datagram telemetry + ACK verification, `URL.canParse`, and 5-Ring Socket & Worker Security Firewall (privileged port `< 1024` block, Cloud Metadata & RFC1918 SSRF block, outbound socket Egress DLP block, concurrency/byte budget guard, and child worker OCap escalation prevention).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"22-worker-threads-and-raw-sockets","auditSummary":{"totalCasesEvaluated":12,"ring1PrivilegedPortBlocked":2,"ring2SsrfIpBlocked":4,"ring3EgressDlpBlocked":2,"ring4BudgetOverflowBlocked":1,"ring5WorkerEscalationBlocked":1,"cleanOperationsAllowed":2,"urlCanParseVerified":true,"auditDigest":"2723413bc42cf3c6"},"sessionSummary":{"workersSpawned":3,"workerTasksCompleted":3,"totalRecordsProcessed":18,"combinedWeightedSum":3475,"combinedXorChecksum":37,"messageChannelSyncVerified":true,"tcpCommandsSent":4,"tcpRepliesVerified":4,"tcpStatusValue":"active-wave-c","udpDatagramsSent":1,"udpAckBytesReceived":30,"udpAckVerified":true,"sessionDigest":"68aebc6ff5256024"}}
  ```

### 23. [`apps/23-wasm-crypto-and-simd-kernel/`](./apps/23-wasm-crypto-and-simd-kernel/src/index.ts) — Pure-Rust `WebAssembly` (`wasmi`) Compute Kernel, Shared Linear Memory, Host Import Callbacks & 5-Ring WASM Security Firewall (`bytes@3.1.2`)
- **Structure:** [`package.json`](./apps/23-wasm-crypto-and-simd-kernel/package.json), [`slice.toml`](./apps/23-wasm-crypto-and-simd-kernel/slice.toml), [`src/types.ts`](./apps/23-wasm-crypto-and-simd-kernel/src/types.ts), [`src/wasm_kernels.ts`](./apps/23-wasm-crypto-and-simd-kernel/src/wasm_kernels.ts), [`src/wasm_security_auditor.ts`](./apps/23-wasm-crypto-and-simd-kernel/src/wasm_security_auditor.ts), [`src/index.ts`](./apps/23-wasm-crypto-and-simd-kernel/src/index.ts).
- **What it tests:** Full W3C/WinterTC `WebAssembly` lifecycle (`WebAssembly.validate`, `new WebAssembly.Module`, `Module.exports`, `Module.imports`, `Module.customSections`, `WebAssembly.instantiate`, `WebAssembly.Memory.prototype.grow`, bidirectional `Uint8Array` linear memory mutation, `WebAssembly.Global`, and JS host function callbacks via `importObject.env`) paired with the **5-Ring WebAssembly Security Firewall** (Ring 1 invalid magic header `CompileError`, Ring 2 deterministic CPU fuel bomb preemption, Ring 3 memory page ceiling enforcement, Ring 4 unlinked host import `LinkError`, and Ring 5 out-of-bounds linear memory `RuntimeError`), plus `bytes` formatting and `node:crypto` SHA-256 verification.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"23-wasm-crypto-and-simd-kernel","kernelSummary":{"validBinaryAccepted":true,"invalidBinaryRejected":true,"exportNames":["memory","ops_counter","fnv1a_32","xor_stream_transform"],"importNames":["env.host_mix"],"customSectionPayload":"slice-wasm-kernel-v1","fnv1aDigestHex":"de32fb41","transformedSliceHex":"c6f7e1c3ecffdb8a8fdcebe089d8ebef","hostCallbackCount":1,"opsCounterAfterRun":49,"memoryPagesAfterGrow":2,"memoryByteLengthFormatted":"128KB","kernelSha256":"c551c0df9a3ec1ef"},"securityAudit":{"totalVectorsTested":5,"ring1MalformedMagicBlocked":true,"ring2CpuFuelBombBlocked":true,"ring3MemoryQuotaExceededBlocked":true,"ring4UnlinkedHostImportBlocked":true,"ring5OutOfBoundsMemoryTrapBlocked":true,"auditDigest":"dc1da3b83df5fa6e"}}
  ```

### 24. [`apps/24-tsx-ssr-and-wasm-edge-pipeline/`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/src/index.ts) — WebAssembly SLA Health Scorer, `WebAssembly.Table`, & XSS-Safe SSR Component Pipeline (`bytes@3.1.2`, `cookie@0.6.0`, `fast-querystring@1.1.2`, `ms@2.1.3` + `slice test --watch-once`)
- **Structure:** [`package.json`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/package.json), [`slice.toml`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/slice.toml), [`src/types.ts`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/src/types.ts), [`src/wasm_scorer.ts`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/src/wasm_scorer.ts), [`src/ssr_renderer.ts`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/src/ssr_renderer.ts), [`src/index.ts`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/src/index.ts), [`tests/ssr_wasm.test.ts`](./apps/24-tsx-ssr-and-wasm-edge-pipeline/tests/ssr_wasm.test.ts).
- **What it tests:** Compiles and instantiates a WebAssembly SLA health scorer (`WebAssembly.compile`, `WebAssembly.instantiate`, `WebAssembly.Table`, `WebAssembly.Global`, `Int32Array` view over `exports.memory.buffer`, and `env.on_anomaly` host callback), renders an XSS-escaped Server-Side Rendered (SSR) HTML dashboard (`renderEdgeDashboardSsrHtml`), formats cookies (`cookie.parse` / `cookie.serialize`), parses query strings (`fast-querystring`), formats cache windows (`ms`), and verifies live OS-native file watching (`slice test --watch-once` via `notify`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"24-tsx-ssr-and-wasm-edge-pipeline","sessionTenant":"tenant_admin_09","filterRegion":"global-edge","cacheTtlWindow":"3 minutes","tenantsEvaluated":4,"wasmHostHookCalls":4,"wasmTableLength":3,"wasmGlobalPenaltyPoints":122,"totalWasmScoreSum":4087,"criticalTenantsCount":1,"ssrHtmlByteLength":1406,"xssEscapedVerified":true,"setCookieHeader":"slice_ssr_digest=4087; Path=/; HttpOnly","ssrHtmlSha256":"7f82a97f29ea4f7b"}
  ```

---

## Verified Linux Docker (`x86_64`) Benchmark Results Across All 31 Workloads ([`bench_latest.md`](../bench_latest.md))

### Execution Latency Across All 31 Applications, Frameworks & Case Studies (20-Run Avg / Warm Isolate)

| # | Workload (`tests/apps/`, `tests/frameworks/`, `tests/case_studies/`) | Slice Cold `.cas` (`SLICEBC\x01`) | Slice Standalone Bin (`SLICEBC\x01`) | Slice Warm (`--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v27.0.0-alpha` | Slice Speedup vs Bun / Node.js |
| :- | :--- | :---: | :---: | :---: | :---: | :---: | :---: | :--- |
| **01** | `01-fintech-checkout` (Multi-File TS + SHA-256) | **`6.462 ms`** (`5.81 ms` Min) | **`6.915 ms`** | **`0.970 ms`** | `14.357 ms` | `40.515 ms` | `64.388 ms` | **`2.22x Bun` (`14.8x` Warm) / `9.96x Node`** |
| **02** | `02-npm-ecosystem-suite` (`semver` + `ms` + `ini`) | **`14.045 ms`** (`13.14 ms` Min) | **`14.795 ms`** | **`4.101 ms`** | `21.324 ms` | `52.462 ms` | `70.900 ms` | **`1.52x Bun` (`5.20x` Warm) / `5.05x Node`** |
| **03** | `03-disk-io-log-pipeline` (10 JSONL Shards Disk I/O) | **`7.891 ms`** (`7.32 ms` Min) | **`8.480 ms`** | **`2.044 ms`** | `14.361 ms` | `42.519 ms` | `63.433 ms` | **`1.82x Bun` (`7.03x` Warm) / `8.04x Node`** |
| **04** | `04-edge-api-gateway` (`fast-querystring` + `ms`) | **`7.793 ms`** (`7.36 ms` Min) | **`8.387 ms`** | **`1.949 ms`** | `14.829 ms` | `41.315 ms` | `63.293 ms` | **`1.90x Bun` (`7.61x` Warm) / `8.12x Node`** |
| **05** | `05-jwt-oauth-session-guard` (`cookie` + `ms` + HMAC) | **`10.875 ms`** (`10.36 ms` Min) | **`11.395 ms`** | **`5.157 ms`** | `15.178 ms` | `49.432 ms` | `65.323 ms` | **`1.40x Bun` (`2.94x` Warm) / `6.01x Node`** |
| **06** | `06-telemetry-metrics-pipeline` (`bytes` + `flatted` + WAL) | **`14.513 ms`** (`13.80 ms` Min) | **`15.101 ms`** | **`5.815 ms`** | `16.831 ms` | `45.490 ms` | `65.159 ms` | **`1.16x Bun` (`2.89x` Warm) / `4.49x Node`** |
| **07** | `07-graphql-ast-validator` (`dlv` + `dset` + AST Cost) | **`9.282 ms`** (`8.86 ms` Min) | **`9.871 ms`** | **`4.160 ms`** | `13.126 ms` | `46.870 ms` | `61.297 ms` | **`1.41x Bun` (`3.16x` Warm) / `6.60x Node`** |
| **08** | `08-event-driven-state-journal` (`mitt` + `ms` + JSONL Replay) | **`11.336 ms`** (`10.72 ms` Min) | **`12.380 ms`** | **`5.362 ms`** | `16.258 ms` | `46.551 ms` | `62.180 ms` | **`1.43x Bun` (`3.03x` Warm) / `5.49x Node`** |
| **09** | `09-wintertc-web-router` (`Request` + `Response` + `URL`, 80 reqs) | `15.435 ms` | **`15.418 ms`** (`14.09 ms` Min) | **`9.402 ms`** | `13.766 ms` | `47.303 ms` | `84.676 ms` | **`1.46x Bun Warm` / `5.49x Node`** |
| **10** | `10-fullstack-bff-microservice` (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **`12.988 ms`** (`12.16 ms` Min) | `13.701 ms` | **`6.387 ms`** | `14.099 ms` | `46.932 ms` | `83.650 ms` | **`1.09x Bun` (`2.21x` Warm) / `6.44x Node`** |
| **11** | `11-static-asset-cdn-edge` (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **`9.831 ms`** (`9.21 ms` Min) | **`10.431 ms`** | **`3.575 ms`** | `13.202 ms` | `52.120 ms` | `86.887 ms` | **`1.34x Bun` (`3.69x` Warm) / `8.84x Node`** |
| **12** | `12-ai-sse-token-streamer` (`mitt` + `ms` + SSE + DLP, 320 frames) | **`16.268 ms`** (`15.38 ms` Min) | **`17.053 ms`** | **`8.443 ms`** | `15.734 ms` | `49.942 ms` | `89.397 ms` | **`1.86x Bun Warm` / `5.50x Node`** |
| **13** | `13-webhook-orchestrator-ssrf` (`fast-querystring` + `ms` + Live `fetch`) | **`13.596 ms`** (`12.97 ms` Min) | **`14.157 ms`** | **`6.651 ms`** | `19.532 ms` | `51.076 ms` | `111.596 ms` | **`1.44x Bun` (`2.94x` Warm) / `8.21x Node`** |
| **14** | `14-federated-api-mesh-dlp` (`bytes` + `dlv` + `dset` + Live `fetch`) | **`14.463 ms`** (`13.86 ms` Min) | **`15.599 ms`** | **`8.065 ms`** | `21.703 ms` | `52.931 ms` | `109.095 ms` | **`1.50x Bun` (`2.69x` Warm) / `7.54x Node`** |
| **15** | `15-time-travel-test-suite` (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **`14.976 ms`** (`14.43 ms` Min) | **`15.350 ms`** | **`5.118 ms`** | `24.472 ms` | `65.626 ms` | `82.490 ms` | **`1.63x Bun` (`4.78x` Warm) / `5.51x Node`** |
| **16** | `16-dotenv-dlp-config-vault` (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **`7.580 ms`** (`7.13 ms` Min) | **`8.043 ms`** | **`1.664 ms`** | `14.572 ms` | `43.637 ms` | `65.300 ms` | **`1.92x Bun` (`8.76x` Warm) / `8.61x Node`** |
| **17** | `17-zero-trust-redirect-keepalive-mesh` (Keep-Alive/307/SSRF/DLP) | **`15.683 ms`** (`14.99 ms` Min) | **`16.255 ms`** | **`8.442 ms`** | `21.812 ms` | `54.938 ms` | `110.915 ms` | **`1.39x Bun` (`2.58x` Warm) / `7.07x Node`** |
| **18** | `18-async-event-loop-fanout-scheduler` (Timer Wheel/`Promise.all`/`AbortSignal`) | **`29.056 ms`** (`27.75 ms` Min) | **`29.790 ms`** | **`17.136 ms`** | `32.800 ms` | `70.636 ms` | `81.020 ms` | **`1.13x Bun` (`1.91x` Warm) / `2.79x Node`** |
| **19** | `19-wintertc-webcrypto-streams-edge` (WebCrypto/Streams/FormData/Clone) | **`8.116 ms`** (`7.44 ms` Min) | **`8.786 ms`** | **`1.722 ms`** | `13.853 ms` | `45.039 ms` | `85.490 ms` | **`1.71x Bun` (`8.04x` Warm) / `10.53x Node`** |
| **20** | `20-node-compat-and-edge-router` (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **`10.578 ms`** (`10.13 ms` Min) | **`11.315 ms`** | **`0.606 ms`** | `22.162 ms` | `54.876 ms` | `71.146 ms` | **`2.10x Bun` (`36.6x` Warm) / `6.73x Node`** |
| **21** | `21-websocket-realtime-and-security-guard` (RFC 6455 `WebSocket` + 5-Ring Guard) | **`10.727 ms`** (`10.07 ms` Min) | **`10.911 ms`** | **`2.504 ms`** | `19.980 ms` | `51.863 ms` | `114.930 ms` | **`1.86x Bun` (`7.98x` Warm) / `10.71x Node`** |
| **22** | `22-worker-threads-and-raw-sockets` (3 Worker Threads + TCP/UDP + 5-Ring Sec) | **`19.960 ms`** (`19.31 ms` Min) | **`20.390 ms`** | **`2.160 ms`** | `54.184 ms` | `110.814 ms` | `99.212 ms` | **`2.71x Bun` (`25.1x` Warm) / `4.97x Node`** |
| **23** | `23-wasm-crypto-and-simd-kernel` (Pure-Rust `WebAssembly` + 5-Ring WASM Sec) | **`16.511 ms`** (`15.77 ms` Min) | **`16.966 ms`** | **`1.449 ms`** | `20.779 ms` | `40.072 ms` | `59.115 ms` | **`1.26x Bun` (`14.3x` Warm) / `3.58x Node`** |
| **24** | `24-tsx-ssr-and-wasm-edge-pipeline` (`WebAssembly` SLA Scorer + SSR Pipeline) | **`8.646 ms`** (`7.79 ms` Min) | **`8.948 ms`** | **`1.747 ms`** | `18.867 ms` | `42.100 ms` | `65.406 ms` | **`2.18x Bun` (`10.8x` Warm) / `7.56x Node`** |
| **25** | `FW-01: frameworks/01-hono-edge-microservice` (`hono@4.6.20` + `hono/cookie` + `ms`) | **`14.577 ms`** (`13.82 ms` Min) | **`15.145 ms`** | **`5.652 ms`** | `19.203 ms` | `51.451 ms` | `93.418 ms` | **`1.32x Bun` (`3.40x` Warm) / `6.41x Node`** |
| **26** | `FW-02: frameworks/02-itty-router-unjs-nitro-stack` (`itty-router` + UnJS) | **`17.079 ms`** (`16.30 ms` Min) | **`17.628 ms`** | **`9.078 ms`** | `19.205 ms` | `46.065 ms` | `89.889 ms` | **`1.12x Bun` (`2.12x` Warm) / `5.26x Node`** |
| **26b** | `FW-03: frameworks/03-node-http-express-compat` (`node:http` + `node:stream` Express/Polka) | **Verified** | **Verified** | **Verified** | **Verified** | **Verified** | **Verified** | **100% 4-Runtime Output Parity (`eb7f023c67427dc4`)** |
| **27** | `CS-01: case_studies/01-tot-autonomous-sre-agent` (20 ToT Sandboxes) | **`26.682 ms`** (`25.20 ms` Min) | `27.212 ms` | **`12.714 ms`** | `28.814 ms` | `68.862 ms` | `86.661 ms` | **`1.08x Bun` (`2.27x` Warm) / `3.25x Node`** |
| **28** | `CS-02: case_studies/02-mcp-code-mode-orchestrator` (`-96.7%` Tokens) | **`19.081 ms`** (`18.48 ms` Min) | **`19.823 ms`** | **`8.094 ms`** | `16.056 ms` | `42.538 ms` | `64.045 ms` | **`1.98x Bun Warm` / `3.36x Node`** |
| **29** | `CS-03: case_studies/03-multi-tenant-saas-plugin-mesh` (12 Tenants + Hot-Swap) | **`13.198 ms`** (`12.54 ms` Min) | **`13.986 ms`** | **`5.702 ms`** | `16.135 ms` | `47.442 ms` | `83.575 ms` | **`1.22x Bun` (`2.83x` Warm) / `6.33x Node`** |
| **30** | `CS-04: case_studies/04-wasm-zero-trust-cdr-pipeline` (Wasm SimHash CDR) | **`17.055 ms`** (`16.22 ms` Min) | **`17.557 ms`** | **`2.019 ms`** | `18.790 ms` | `43.061 ms` | `66.076 ms` | **`1.10x Bun` (`9.31x` Warm) / `3.87x Node`** |
| **31** | `CS-05: case_studies/05-time-travel-chaos-fuzzer-payments` (48 Payment Sagas) | **`17.484 ms`** (`16.92 ms` Min) | **`18.490 ms`** | **`9.455 ms`** | `19.619 ms` | `61.474 ms` | `82.259 ms` | **`1.12x Bun` (`2.08x` Warm) / `4.70x Node`** |

### Peak Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`SLICEBC\x01` Default) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v27.0.0-alpha`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **App 2 (`02-npm-ecosystem-suite`: `semver`+`ms`+`ini`)** | **13,824 KB (13.50 MB)** | 30,200 KB (29.49 MB) | 47,100 KB (46.00 MB) | 71,368 KB (69.70 MB) | **5.16x less than Node / 2.18x less than Bun** |
| **App 6 (`06-telemetry-metrics-pipeline`: `bytes`+`flatted`)** | **14,328 KB (13.99 MB)** | 30,592 KB (29.88 MB) | 51,184 KB (49.98 MB) | 72,732 KB (71.03 MB) | **5.08x less than Node / 2.14x less than Bun** |
| **App 21 (`21-websocket-realtime-and-security-guard`)** | **13,952 KB (13.62 MB)** | 31,700 KB (30.96 MB) | 50,788 KB (49.60 MB) | 94,976 KB (92.75 MB) | **6.81x less than Node / 2.27x less than Bun** |
| **App 22 (`22-worker-threads-and-raw-sockets`: 3 Workers + TCP/UDP)** | **21,888 KB (21.38 MB)** | 41,188 KB (40.22 MB) | 63,308 KB (61.82 MB) | 93,320 KB (91.13 MB) | **4.26x less than Node / 1.88x less than Bun** |
| **App 23 (`23-wasm-crypto-and-simd-kernel`: Pure-Rust WASM)** | **18,944 KB (18.50 MB)** | 31,104 KB (30.38 MB) | 49,048 KB (47.90 MB) | 80,276 KB (78.39 MB) | **4.24x less than Node / 1.64x less than Bun** |
| **FW 1 (`frameworks/01-hono-edge-microservice`: `hono` v4)** | **14,848 KB (14.50 MB)** | 28,408 KB (27.74 MB) | 48,996 KB (47.85 MB) | 89,628 KB (87.53 MB) | **6.04x less than Node / 1.91x less than Bun** |
| **FW 2 (`frameworks/02-itty-router-unjs-nitro-stack`: UnJS / Nitro)** | **14,080 KB (13.75 MB)** | 32,000 KB (31.25 MB) | 48,724 KB (47.58 MB) | 87,176 KB (85.13 MB) | **6.19x less than Node / 2.27x less than Bun** |
| **CS 1–5 (`case_studies/01..05`: ToT Agent, MCP, SaaS Mesh, Wasm CDR, Chaos Sagas)** | **14.25 MB – 17.00 MB** | 26.00 MB – 31.12 MB | 45.29 MB – 51.61 MB | 78.63 MB – 85.88 MB | **Up to 6.03x less than Node / 2.18x less than Bun** |

---

## Running the Benchmark Suite (Tagged Filtering via `SLICE_BENCH_TAGS`)

[`run_real_stack_benchmarks.sh`](./run_real_stack_benchmarks.sh) supports filtering benchmark suites via the `SLICE_BENCH_TAGS` environment variable (`all`, `apps`, `frameworks`, `case_studies`, `n1`, `n2`, `n3`, `f1`, `f2`, `f3`, `f4`, `wasm`, `serve`, `swarm`):

| Tag | Suite Executed | Verified Linux Docker (`x86_64`) Highlight (`SLICEBC\x01` Default) |
| :--- | :--- | :--- |
| `apps` | All 24 folder-based TypeScript + `node_modules` apps (`01`..`24`) | **`6.46 ms - 29.06 ms` cold** / **`0.61 ms - 17.14 ms` warm** (`13.50 MB` Max RSS, **`24/24` faster than Bun warm**) |
| `frameworks` | Full-stack Edge & Node Framework suites (`01-hono-edge-microservice`, `02-itty-router-unjs-nitro-stack`, `03-node-http-express-compat`) | **`14.58 ms` Hono (`5.65 ms` warm, `14.50 MB` RSS, `1.32x` Bun)**, **`17.08 ms` UnJS/Nitro (`9.08 ms` warm, `13.75 MB` RSS, `1.12x` Bun)** & **`1.81 ms` warm `node:http` Express/Polka** |
| `case_studies` | All 5 production Case Studies (`01-tot-autonomous-sre-agent`..`05-time-travel-chaos-fuzzer-payments`) | **`13.20 ms - 26.68 ms` cold** / **`2.02 ms - 12.71 ms` warm** (`15.00 MB - 19.00 MB` RSS) |
| `wasm` | Pure-Rust `WebAssembly` (`Apps 23 & 24`) & OS-native `slice test --watch-once` | **`16.51 ms` App 23 (`1.45 ms` warm, `18.50 MB` RSS)** & **`8.65 ms` App 24 (`1.75 ms` warm, `13.50 MB` RSS)** |
| `n1` / `serve` | `slice serve` (`--workers=8`, HTTP/1.1 Keep-Alive + Pipelining + WinterTC) | **`158,534 req/s` (`6.31 us/req`) Tier-0 CAS** / **`120,333 req/s` (`8.31 us/req`) Tier-1 WinterTC** |
| `n2` | Sharded `Slice.kv` (`1,001` ops), `--tenant-dir` hot-swap, & `--deterministic` replay | **`596.53 us` for 1,001 `Slice.kv` ops**, zero-downtime `.cas` hot-swap, & 100% deterministic replay |
| `n3` | Tree-of-Thought AI Agent (`Slice.checkpoint` + `Slice.evalSandbox` + `Slice.restore`), `--audit-egress`, & `slice mcp` | **`107.06 us` for 3 nested sandboxes + VFS/KV rollback (`15.22 us/sandbox`)**, Base64 leak blocked, & JSON-RPC 2.0 MCP tools |
| `f1` | `slice serve` with `export default { fetch(req, env, ctx) }` + App 9 & App 10 WinterTC parity | **`export default { fetch }` HTTP server + 100% 4-runtime parity on Apps 9 & 10 (`12.99 ms` on App 10 vs `14.10 ms` Bun)** |
| `f2` | `--static` Zero-Stat CAS Static Asset ETag (`HIT-200` + `HIT-304`) & `Response.sse` + Apps 11 & 12 | **`< 2 us` BLAKE3 `ETag` `304 Not Modified` + SSE token streaming + 100% 4-runtime parity on Apps 11 & 12** |
| `f3` | OCap Outbound `fetch()`, Cloud Metadata SSRF Shield, & Outbound Egress DLP + Apps 13, 14 & 17 | **Live TCP `fetch()` (`13.60 ms` App 13 vs `19.53 ms` Bun, `14.46 ms` App 14 vs `21.70 ms` Bun) + `169.254.169.254` SSRF & Base64 DLP blocked** |
| `f4` | Time-Travel `slice test`, `< 3 ms` `--watch`, Auto-DLP `.env`, & Event Loop + Apps 15, 16 & 18 | **Time-Travel `slice test` (`3/3 passed`, `3/3 VFS+KV rollbacks`) + `14.98 ms` App 15 (`1.63x` Bun) & `7.58 ms` App 16 (`1.92x` Bun)** |
| `swarm` | 1,000-Tenant MicroVM Cold-Start & Density Benchmark (`slice microvm-bench --tenants=1000`) | **`0.98 us` min / `1.03 us` P50 / `1.28 ms` total 1,000-tenant swarm (`10.06 MB` process RSS, 5/5 security vectors blocked)** |

```powershell
# Build the Linux Docker benchmark container
docker build -f tests/Dockerfile -t slice-realworld-bench .

# Run all suites (default: SLICE_BENCH_TAGS=all)
docker run --rm slice-realworld-bench

# Or run only specific tagged suites (e.g. f1, n1, n2, n3, and 1,000-tenant swarm)
docker run --rm -e SLICE_BENCH_TAGS=f1,n1,n2,n3,swarm slice-realworld-bench

# Or run any of the 20 folder apps directly on Windows or Linux with Slice
.\target\release\slice.exe run .\tests\apps\01-fintech-checkout --warm --metrics
.\target\release\slice.exe run .\tests\apps\02-npm-ecosystem-suite --warm --metrics
.\target\release\slice.exe run .\tests\apps\19-wintertc-webcrypto-streams-edge --warm --metrics
.\target\release\slice.exe run .\tests\apps\20-node-compat-and-edge-router --warm --metrics
```
```
