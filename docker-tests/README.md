# Slice Docker-Based Folder Applications, External NPM Libraries & Disk I/O Benchmark Suite

This directory (`docker-tests/`) contains a reproducible Linux `x86_64` Docker benchmark suite comparing **Slice (Nexus-JS)** against the **absolute latest releases** of **Bun (`v1.4.4`)**, **Deno (`v2.9.8`)**, and **Node.js (`v26.12.1`)** across **20 folder-based production TypeScript applications** (`docker-tests/apps/`).

---

## Latest Runtimes Installed in Docker ([`docker-tests/Dockerfile`](file:///p:/opensource/slice/docker-tests/Dockerfile))

- **Slice (`v0.2.0`):** Hybrid architecture combining **Oxc** (`v0.154.0` production Rust TypeScript compiler + AST ESM-to-CJS module bundler + full `package.json` `"exports"` resolver + static barrel tree-shaker), **Boa (`boa_engine` v0.22)** full ECMAScript VM with zero-copy `SLICEBOA` binary payloads, thread-local pre-warmed context pool (`BOA_CONTEXT_POOL`), native Rust fast-path intrinsics (`SemVer`, `SHA-256`/`SHA-384`/`SHA-512`, `HMAC-SHA256`/`HMAC-SHA512`, `Base64URL`, in-memory `VFS` slab cache, WinterTC `URL`/`URLSearchParams`/`Headers`/`Request`/`Response`/`ReadableStream`/`TransformStream`/`TextEncoder`/`TextDecoder`/`crypto.subtle`/`Blob`/`File`/`FormData`/`structuredClone`/`EventTarget`/`AbortSignal`, real `Uint8Array`-backed `Buffer`, `node:*` Core Tier-1 modules, `Slice.streamSSE` / `Response.sse`, OCap-guarded outbound HTTP/1.1 & HTTPS TLS 1.3 `fetch()` with Keep-Alive pooling, hop-by-hop redirect guard, Cloud Metadata SSRF Shield, and Egress DLP, built-in Time-Travel `slice test` runner, `< 3 ms` `--watch`, and Auto-DLP `.env`), and **Slice Zero-Stat CAS + `--static` BLAKE3 `ETag` Static Server + Tier-0/Tier-2 fast paths**.
- **Node.js (`v26.12.1` — Absolute Latest Release):** Dynamically resolved at container build time from `https://nodejs.org/dist/index.json` (`.[0].version`).
- **Bun (`v1.4.4` — Absolute Latest Release):** Installed via official `https://bun.sh/install`.
- **Deno (`v2.9.8` — Absolute Latest Release):** Installed via official `https://deno.land/install.sh`.

---

## 20 Folder-Based Production Test Applications (`docker-tests/apps/`)

### 1. [`apps/01-fintech-checkout/`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/index.ts) — Multi-Module TypeScript E-Commerce & SHA-256 Crypto Signing
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/types.ts), [`src/audit_crypto.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/audit_crypto.ts), [`src/pricing_engine.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/pricing_engine.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/01-fintech-checkout/src/index.ts).
- **What it tests:** Multi-file TypeScript interfaces, union types, `PricingAndRiskEngine` class, regional VAT/sales tax, tiered + coupon discounts, velocity/geo/GPU fraud risk heuristics, and FIPS-180-4 SHA-256 receipt signing via `node:crypto` across 50 checkout orders.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"01-fintech-checkout","ordersProcessed":50,"approvedCount":50,"aggregateRevenueCents":4932364,"lastSha256Signature":"636588368661f40b"}
  ```

### 2. [`apps/02-npm-ecosystem-suite/`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/index.ts) — Real External NPM Libraries (`semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/package.json), [`src/release_planner.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/release_planner.ts), [`src/config_loader.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/config_loader.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/02-npm-ecosystem-suite/src/index.ts).
- **What it tests:** Static barrel tree-shaking and execution of real third-party CommonJS/ESM packages from `node_modules/` (`semver.valid`, `semver.coerce`, `semver.satisfies`, `semver.gt`, `ms`, and `ini.parse`) across 20 microservice release manifests and cluster SLA topologies, signed with `node:crypto` SHA-256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"02-npm-ecosystem-suite","libraries":["semver@7.6.3","ms@2.1.3","ini@4.1.3"],"validManifests":20,"rangeSatisfied":5,"upgradesRequired":20,"highestSemver":"5.0.0","totalSlaTimeoutMs":197500,"edgeCacheTtlMs":7200000,"heartbeatInterval":"2 minutes","sha256Digest":"ef97948c06f43da9"}
  ```

### 3. [`apps/03-disk-io-log-pipeline/`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/index.ts) — High-Throughput Disk I/O, Directory Scan & JSONL Stream Processing
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/package.json), [`src/log_generator.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/log_generator.ts), [`src/io_aggregator.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/io_aggregator.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/03-disk-io-log-pipeline/src/index.ts).
- **What it tests:** Intensive filesystem I/O (`node:fs`, `node:path`, `node:os`, `node:crypto`) creating, writing, stating, directory-scanning, reading, parsing, SHA-256 verifying, and unlinking 10 JSONL log shard files (`200` records, `15,796 bytes`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"03-disk-io-log-pipeline","shardsScanned":10,"recordsParsed":200,"totalDiskBytes":15796,"error5xxCount":28,"rateLimit429Count":28,"totalBytesTransferred":851796,"maxLatencyMs":121,"summarySha256":"4cbda5d6e5f1f7ca"}
  ```

### 4. [`apps/04-edge-api-gateway/`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/index.ts) — Edge API Gateway with External NPM Query Parser (`fast-querystring@1.1.2`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/package.json), [`src/auth_middleware.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/auth_middleware.ts), [`src/rate_limiter.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/rate_limiter.ts), [`src/router.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/router.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/04-edge-api-gateway/src/index.ts).
- **What it tests:** URL query parsing with Fastify's `fast-querystring` library, sliding-window tenant quota enforcement with `ms`, Bearer token authentication, SHA-256 audit tagging, and route dispatching over 100 HTTP requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"04-edge-api-gateway","libraries":["fast-querystring@1.1.2","ms@2.1.3"],"totalRequests":100,"status200":75,"status401":25,"status429":0,"status404":0,"totalResponseBytes":6700,"latencyPoints":300,"gatewaySha256":"552ff4506f3699d0"}
  ```

### 5. [`apps/05-jwt-oauth-session-guard/`](file:///p:/opensource/slice/docker-tests/apps/05-jwt-oauth-session-guard/src/index.ts) — Zero-Trust OAuth2 / JWT HMAC-SHA256 & HTTP Cookie Session Guard (`cookie@0.6.0`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/05-jwt-oauth-session-guard/package.json), [`src/jwt_codec.ts`](file:///p:/opensource/slice/docker-tests/apps/05-jwt-oauth-session-guard/src/jwt_codec.ts), [`src/session_manager.ts`](file:///p:/opensource/slice/docker-tests/apps/05-jwt-oauth-session-guard/src/session_manager.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/05-jwt-oauth-session-guard/src/index.ts).
- **What it tests:** RFC 6265 HTTP `Cookie` / `Set-Cookie` parsing and serialization (`cookie.parse`, `cookie.serialize`), Base64URL JWT encoding/decoding (`Buffer.from(..., "base64url")`), RFC 2104 HMAC-SHA256 signature verification (`crypto.createHmac("sha256", secret)`), CSRF double-submit token validation, and RBAC scope authorization across 50 sessions.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"05-jwt-oauth-session-guard","libraries":["cookie@0.6.0","ms@2.1.3"],"sessionsEvaluated":50,"authenticatedCount":45,"csrfVerifiedCount":38,"rbacGrantedCount":12,"deniedCount":38,"totalCookieBytes":5448,"auditSha256":"6c262153f1280c9e"}
  ```

### 6. [`apps/06-telemetry-metrics-pipeline/`](file:///p:/opensource/slice/docker-tests/apps/06-telemetry-metrics-pipeline/src/index.ts) — OpenTelemetry Circular Span Graph, Flatted Serialization & Disk WAL (`bytes@3.1.2`, `flatted@3.3.1`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/06-telemetry-metrics-pipeline/package.json), [`src/span_graph.ts`](file:///p:/opensource/slice/docker-tests/apps/06-telemetry-metrics-pipeline/src/span_graph.ts), [`src/histogram_wal.ts`](file:///p:/opensource/slice/docker-tests/apps/06-telemetry-metrics-pipeline/src/histogram_wal.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/06-telemetry-metrics-pipeline/src/index.ts).
- **What it tests:** Constructing parent<->child circular distributed trace span graphs (`SpanNode`), serializing and deserializing cyclic graphs with `flatted` (`stringify` / `parse`), human-readable memory budget formatting/parsing with `bytes` (`bytes.parse`, `bytes.format`), P50/P95 latency histogram calculation, and WAL persistence (`node:fs` + `node:crypto`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"06-telemetry-metrics-pipeline","libraries":["bytes@3.1.2","flatted@3.3.1"],"tracesProcessed":20,"spansEvaluated":60,"p50DurationUs":949,"p95DurationUs":1072,"totalMemoryFormatted":"4.56MB","walBytesOnDisk":11827,"walSha256":"c93139476b4093bb"}
  ```

### 7. [`apps/07-graphql-ast-validator/`](file:///p:/opensource/slice/docker-tests/apps/07-graphql-ast-validator/src/index.ts) — GraphQL Query AST Depth/Complexity Cost Guard & Deep Resolver (`dlv@1.1.3`, `dset@3.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/07-graphql-ast-validator/package.json), [`src/graphql_ast.ts`](file:///p:/opensource/slice/docker-tests/apps/07-graphql-ast-validator/src/graphql_ast.ts), [`src/deep_resolver.ts`](file:///p:/opensource/slice/docker-tests/apps/07-graphql-ast-validator/src/deep_resolver.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/07-graphql-ast-validator/src/index.ts).
- **What it tests:** Recursive GraphQL selection-set AST depth and complexity cost analysis (blocking abusive deep queries), paired with deep nested object resolution and projection using `dlv` and `dset` over 60 GraphQL queries.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"07-graphql-ast-validator","libraries":["dlv@1.1.3","dset@3.1.3"],"queriesEvaluated":60,"allowedQueries":48,"blockedDeepQueries":12,"totalComplexityScore":2700,"totalProjectedBytes":11472,"schemaSha256":"48510bb0a32a884d"}
  ```

### 8. [`apps/08-event-driven-state-journal/`](file:///p:/opensource/slice/docker-tests/apps/08-event-driven-state-journal/src/index.ts) — CQRS Event-Sourced Ledger Bus & Append-Only JSONL WAL Replay (`mitt@3.0.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/08-event-driven-state-journal/package.json), [`src/event_ledger.ts`](file:///p:/opensource/slice/docker-tests/apps/08-event-driven-state-journal/src/event_ledger.ts), [`src/journal_replayer.ts`](file:///p:/opensource/slice/docker-tests/apps/08-event-driven-state-journal/src/journal_replayer.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/08-event-driven-state-journal/src/index.ts).
- **What it tests:** Pub/sub event-sourced state machine orchestration via `mitt`, append-only write-ahead log (WAL) streaming (`fs.appendFileSync`), full JSONL state replay, and deterministic state convergence verification across 75 ledger events.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"08-event-driven-state-journal","libraries":["mitt@3.0.1","ms@2.1.3"],"eventsDispatched":75,"appliedCount":41,"rejectedCount":34,"replayedAggregateBalanceCents":95480,"journalDiskBytes":10699,"stateMatchVerified":true,"journalSha256":"4a603ab1b248253a"}
  ```

### 9. [`apps/09-wintertc-web-router/`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/src/index.ts) — Standard WinterTC Web API (`Request`, `Response`, `Headers`, `URL`, `URLSearchParams`) Edge Router
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/src/types.ts), [`src/catalog.ts`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/src/catalog.ts), [`src/router.ts`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/src/router.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/09-wintertc-web-router/src/index.ts).
- **What it tests:** Standard WinterTC / WHATWG `new Request(url, { method, headers, body })`, `new URL(req.url)`, `url.searchParams.get` / `.getAll` / `.sort`, case-insensitive `Headers`, `Response.json(data, init)`, `Response.redirect(url, 308)`, `await res.json()`, and `node:crypto` HMAC-SHA256 webhook verification across 80 multi-tenant HTTP requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"09-wintertc-web-router","totalRequests":80,"ok200Count":30,"created201Count":16,"redirect308Count":10,"unauthorized401Count":10,"notFound404Count":14,"totalRevenueCents":5255850,"matchedCatalogItems":30,"canonicalQuerySample":"category=compute&minPrice=0&sort=asc&tag=pcie","finalStateHash":"bda3093620e333db"}
  ```

### 10. [`apps/10-fullstack-bff-microservice/`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/src/index.ts) — Full-Stack Multi-Tenant BFF Microservice (`cookie@0.6.0`, `dlv@1.1.3`, `ms@2.1.3` + WinterTC Fetch APIs)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/src/types.ts), [`src/cors_etag.ts`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/src/cors_etag.ts), [`src/bff_controller.ts`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/src/bff_controller.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/10-fullstack-bff-microservice/src/index.ts).
- **What it tests:** Combines standard WinterTC `Request`, `Response`, `Headers`, and `URL` with real external npm packages (`cookie.parse` / `cookie.serialize`, `dlv` deep property resolution, and `ms` cache TTL parsing), `OPTIONS` CORS preflight (`204`), SHA-256 `ETag` conditional `If-None-Match` (`304 Not Modified`), and JSON preference mutations (`201`) across 60 requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"10-fullstack-bff-microservice","totalRequests":60,"corsPreflight204Count":10,"notModified304Count":10,"dashboard200Count":20,"preferences201Count":10,"forbidden403Count":10,"totalWidgetsRendered":80,"totalCacheTtlSec":4200,"finalStateHash":"8b0bf90129dbcc3c"}
  ```

### 11. [`apps/11-static-asset-cdn-edge/`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/src/index.ts) — Zero-Stat Static Asset CDN Edge Router (`bytes@3.1.2`, `ms@2.1.3` + `ETag` `304`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/src/types.ts), [`src/asset_manifest.ts`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/src/asset_manifest.ts), [`src/cdn_server.ts`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/src/cdn_server.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/11-static-asset-cdn-edge/src/index.ts).
- **What it tests:** Static web asset indexing (`.html`, `.css`, `.js`, `.svg`, `.json`), directory index aliases (`/` -> `/index.html`, `/docs/` -> `/docs/index.html`), SHA-256 `ETag` conditional `If-None-Match` (`304 Not Modified`), `HEAD` vs `GET` negotiation, `Cache-Control` TTL calculation via `ms`, and bandwidth-saved formatting via `bytes` across 80 requests.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"11-static-asset-cdn-edge","libraries":["bytes@3.1.2","ms@2.1.3"],"totalRequests":80,"hit200Count":41,"notModified304Count":24,"head200Count":7,"notFound404Count":8,"totalBytesServed":4857,"formattedBandwidthSaved":"2.97KB","manifestDigest":"84d4b4108f88f928"}
  ```

### 12. [`apps/12-ai-sse-token-streamer/`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/src/index.ts) — AI Agent SSE Token Streamer & Outbound Egress DLP (`mitt@3.0.1`, `ms@2.1.3` + `ReadableStream`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/package.json), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/src/types.ts), [`src/sse_codec.ts`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/src/sse_codec.ts), [`src/agent_streamer.ts`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/src/agent_streamer.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/12-ai-sse-token-streamer/src/index.ts).
- **What it tests:** WHATWG Server-Sent Events (`id:`, `event:`, `retry:`, `data:`) token streaming over standard `ReadableStream` (`getReader().read()`), `mitt` pub/sub event bus, and real-time Egress DLP secret redaction (raw + Base64) across 40 AI completion streams (`320` SSE frames).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"12-ai-sse-token-streamer","libraries":["mitt@3.0.1","ms@2.1.3"],"streamsCompleted":40,"totalFramesEmitted":320,"totalTokensStreamed":230,"dlpRedactionsTriggered":10,"totalSseBytes":23410,"heartbeatIntervalMs":15000,"streamAuditHash":"9e53ed87fbc54b01"}
  ```

### 13. [`apps/13-webhook-orchestrator-ssrf/`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/src/index.ts) — Outbound `fetch()` Webhook Orchestrator & Cloud Metadata SSRF Firewall (`fast-querystring@1.1.2`, `ms@2.1.3` + Live TCP `fetch`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/src/types.ts), [`src/ssrf_guard.ts`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/src/ssrf_guard.ts), [`src/webhook_dispatcher.ts`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/src/webhook_dispatcher.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/13-webhook-orchestrator-ssrf/src/index.ts).
- **What it tests:** Outbound `await fetch(url, { method: "POST", headers, body })` against a live upstream HTTP microservice (`127.0.0.1:18095/upstream/webhook`), `fast-querystring` canonicalization, `ms` retry backoff calculation, `node:crypto` HMAC-SHA256 webhook signature generation, and SSRF defense across 40 webhook jobs (blocking AWS/GCP/Alibaba cloud metadata endpoints, unapproved RFC1918 private CIDRs, and untrusted external domains while delivering 10 live HTTP webhooks).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"13-webhook-orchestrator-ssrf","libraries":["fast-querystring@1.1.2","ms@2.1.3"],"totalJobsProcessed":40,"liveHttpDeliveries":10,"ssrfMetadataBlocked":10,"privateCidrBlocked":10,"untrustedDomainBlocked":10,"retryBackoffMs":80000,"auditSignaturePrefix":"db472120dedc36d5"}
  ```

### 14. [`apps/14-federated-api-mesh-dlp/`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/src/index.ts) — Federated Subgraph API Mesh & Outbound Egress DLP Firewall (`bytes@3.1.2`, `dlv@1.1.3`, `dset@3.1.3` + Live TCP `fetch`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/src/types.ts), [`src/dlp_firewall.ts`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/src/dlp_firewall.ts), [`src/mesh_federator.ts`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/src/mesh_federator.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/14-federated-api-mesh-dlp/src/index.ts).
- **What it tests:** Outbound `await fetch("http://127.0.0.1:18095/upstream/subgraph/account?...")` subgraph fan-out across 25 federated queries, deep object extraction/stitching with `dlv` and `dset`, payload formatting via `bytes`, and Outbound Egress DLP inspection (catching raw, Base64, and Base64URL secret exfiltration in URLs, headers, and bodies).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"14-federated-api-mesh-dlp","libraries":["bytes@3.1.2","dlv@1.1.3","dset@3.1.3"],"queriesExecuted":25,"subgraphFetchesCompleted":25,"dlpExfiltrationsBlocked":5,"cleanAuditForwards":20,"totalAggregatedBytes":"3.2KB","meshStateHash":"973215696f65c8cd"}
  ```

### 15. [`apps/15-time-travel-test-suite/`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/src/index.ts) — Time-Travel VFS + KV Rollback Verification Suite (`semver@7.6.3`, `ms@2.1.3`, `dlv@1.1.3`, `dset@3.1.3` + `slice test`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/src/types.ts), [`src/state_machine.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/src/state_machine.ts), [`src/time_travel_harness.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/src/time_travel_harness.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/src/index.ts), [`tests/state_machine.test.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/tests/state_machine.test.ts).
- **What it tests:** Executes 24 state-mutating specifications across 4 suites (`semver.satisfies`, `ms`, `dlv`, `dset`, `node:fs` VFS mutations, and `Slice.kv` balance/version mutations), taking a `< 5 us` `Slice.checkpoint()` before each spec and restoring it via `Slice.restore(snapId)` in `finally` to verify 24/24 zero-leakage VFS and KV state rollbacks. Also includes a native [`tests/state_machine.test.ts`](file:///p:/opensource/slice/docker-tests/apps/15-time-travel-test-suite/tests/state_machine.test.ts) verified via `slice test`.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"15-time-travel-test-suite","libraries":["dlv@1.1.3","dset@3.1.3","ms@2.1.3","semver@7.6.3"],"suitesExecuted":4,"specsExecuted":24,"specsPassed":24,"vfsRollbacksVerified":24,"kvRollbacksVerified":24,"totalRetentionMs":29462400000,"suiteAuditHash":"c624c2019b5c44fd"}
  ```

### 16. [`apps/16-dotenv-dlp-config-vault/`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/src/index.ts) — Zero-Trust `.env` Config Vault & Auto-DLP Secret Exfiltration Auditor (`ini@4.1.3`, `cookie@0.6.0`, `bytes@3.1.2`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/src/types.ts), [`src/dotenv_vault.ts`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/src/dotenv_vault.ts), [`src/auto_dlp_auditor.ts`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/src/auto_dlp_auditor.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/16-dotenv-dlp-config-vault/src/index.ts).
- **What it tests:** Parses `.env` configuration files (`KEY=VALUE`, `export KEY="VALUE"`, inline comments) alongside `ini` cluster limits, `bytes` payload budgets, and `cookie.serialize` headers, automatically classifying sensitive keys (`STRIPE_SECRET_KEY`, `OPENAI_API_TOKEN`, `DB_MASTER_PASSWORD`, `SIGNING_PRIVATE_KEY`) vs public keys (`PUBLIC_KEY_ID`, `PUBLISHABLE_KEY_WEB`) and auditing 50 outbound telemetry payloads against raw, Base64, and Base64URL secret exfiltration, signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"16-dotenv-dlp-config-vault","libraries":["bytes@3.1.2","cookie@0.6.0","ini@4.1.3"],"dotenvEntriesLoaded":8,"publicKeysAllowed":4,"autoDlpSecretsRegistered":4,"payloadsAudited":50,"cleanPayloadsApproved":40,"plaintextLeaksBlocked":5,"base64LeaksBlocked":5,"totalApprovedCookieBytes":"9.55KB","vaultHmacSignature":"c19618e409a21e7a"}
  ```

### 17. [`apps/17-zero-trust-redirect-keepalive-mesh/`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/src/index.ts) — Zero-Trust Hop-by-Hop Redirect Guard, Keep-Alive Connection Pooling & DNS-Rebinding SSRF Firewall (`bytes@3.1.2`, `cookie@0.6.0`, `fast-querystring@1.1.2`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/src/types.ts), [`src/redirect_security_guard.ts`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/src/redirect_security_guard.ts), [`src/keepalive_mesh_client.ts`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/src/keepalive_mesh_client.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/17-zero-trust-redirect-keepalive-mesh/src/index.ts).
- **What it tests:** Executes 30 zero-trust API mesh tasks against a live HTTP/1.1 Keep-Alive upstream server (`127.0.0.1:18095`), exercising thread-local Keep-Alive connection reuse (`OUTBOUND_CONN_POOL`), `307 Temporary Redirect` hop-by-hop following (`res.redirected === true`, `res.url` tracking), `302 Found` manual redirect inspection (`redirect: "manual"`), cross-origin `Authorization` / `Cookie` stripping, Cloud Metadata (`169.254.169.254`, `fd00:ec2::254`) & DNS-rebinding private IP (`100.64.0.15`) redirect blocking, and Outbound Egress DLP, signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"17-zero-trust-redirect-keepalive-mesh","libraries":["bytes@3.1.2","cookie@0.6.0","fast-querystring@1.1.2","ms@2.1.3"],"tasksProcessed":30,"keepAliveSubgraphHits":5,"redirectFollow307Completed":5,"redirectManual302Inspected":5,"ssrfRedirectMetadataBlocked":5,"dnsRebindPrivateIpBlocked":5,"dlpExfiltrationBlocked":5,"crossOriginAuthStrippedCount":15,"totalTimeoutBudgetMs":50000,"formattedPayloadVolume":"860B","meshAuditHmac":"90f5a8253a5d07de"}
  ```

### 18. [`apps/18-async-event-loop-fanout-scheduler/`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/src/index.ts) — Deterministic Time-Travel Event Loop, Timer Wheel, Microtask Queue, `AbortSignal`, & Concurrent `Promise.all` Fetch Fan-Out (`bytes@3.1.2`, `flatted@3.3.1`, `mitt@3.0.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/src/types.ts), [`src/timer_retry_scheduler.ts`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/src/timer_retry_scheduler.ts), [`src/concurrent_fanout_client.ts`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/src/concurrent_fanout_client.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/18-async-event-loop-fanout-scheduler/src/index.ts).
- **What it tests:** Verifies deterministic microtask/macrotask priority ordering (`Promise.then`, `queueMicrotask`, `setTimeout` with extra arguments, `clearTimeout`, `setInterval`, `clearInterval`), exponential backoff retry scheduling with `ms`, 5 rounds of 5-shard concurrent `Promise.all([fetch(...), ...])` fan-out with `flatted` + `bytes` + `mitt` event bus, `AbortController` / `AbortSignal` task cancellation, and sandbox timer-bomb containment (`MAX_ACTIVE_TIMERS`), signed with `node:crypto` HMAC-SHA256.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"18-async-event-loop-fanout-scheduler","libraries":["bytes@3.1.2","flatted@3.3.1","mitt@3.0.1","ms@2.1.3"],"scheduleOrder":"promise-micro-1>queue-micro-1>timer-stage-a:p1>micro-inside-timer>heartbeat:1>heartbeat:2>heartbeat:3","intervalTicks":3,"cancelledTimerFired":false,"totalRetryBackoffMs":50,"totalShardsExecuted":25,"totalRecordsProcessed":1250,"totalAbortedTasks":5,"totalRetryAttempts":30,"busEventsDispatched":5,"timerBombContained":true,"formattedWireVolume":"1.92KB","auditDigest":"0b76cd366dc934f2"}
  ```

### 19. [`apps/19-wintertc-webcrypto-streams-edge/`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/src/index.ts) — WinterTC Minimum Common Web Platform API: WebCrypto (`crypto.subtle` HMAC/SHA/JWK), `TextEncoder`/`TextDecoder`, WhatWG Streams (`TransformStream`/`TextEncoderStream`), `EventTarget`, `AbortSignal.any`, `Blob`/`File`/`FormData`, & `structuredClone` (`bytes@3.1.2`, `cookie@1.0.2`, `flatted@3.3.1`, `ms@2.1.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/src/types.ts), [`src/webcrypto_jwt_verifier.ts`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/src/webcrypto_jwt_verifier.ts), [`src/streams_and_formdata_pipeline.ts`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/src/streams_and_formdata_pipeline.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/19-wintertc-webcrypto-streams-edge/src/index.ts).
- **What it tests:** Executes a full WinterTC Edge Gateway & Webhook Verification pipeline using `crypto.subtle` (`importKey`, `exportKey` JWK, `sign`, `verify` HS256 & HS512, `digest` SHA-256 & SHA-512), `TextEncoder` (`encode`, `encodeInto` with multi-byte UTF-8 boundary truncation) & `TextDecoder` (`fatal: true`), `btoa`/`atob`, `EventTarget` (`{ once: true, signal }`), `CustomEvent`, `AbortSignal.any`, `Blob` (`.slice()`, `.text()`), `File`, `FormData`, cyclic `structuredClone` (`Map`, `Set`, `Uint8Array`), and a 5-stage WhatWG Stream pipeline (`ReadableStream` -> `TransformStream` -> `TextEncoderStream` -> `TransformStream` -> `TextDecoderStream` -> `WritableStream`).
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"app":"19-wintertc-webcrypto-streams-edge","libraries":["bytes@3.1.2","cookie@1.0.2","flatted@3.3.1","ms@2.1.3"],"hs256TokenValid":true,"hs256TamperedRejected":true,"hs512WebhookValid":true,"jwkAlg":"HS256","encodeIntoRead":8,"encodeIntoWritten":10,"transformedPayload":"ROUTE:API  |AUTH:JWT  |STREAM:OK::EOF","totalTransformedBytes":37,"formFieldsCount":3,"uploadedFileName":"bundle-manifest.bin","uploadedFileSize":38,"slicedHeaderText":"SLICE-EDGE","cyclicCloneVerified":true,"dispatchedEvents":["once:init","live:init","live:transform"],"anySignalAbortedBy":"secondary-deadline-exceeded","formattedWireVolume":"75B","auditDigest":"7319225b3dc10146"}
  ```

### 20. [`apps/20-node-compat-and-edge-router/`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/src/index.ts) — Real `Uint8Array`-Backed `Buffer` Wire Framing, `node:*` Core Tier-1 (`node:events`, `node:path`, `node:crypto`, `node:assert`, `node:util`), & Oxc AST Bundler (`bytes@3.1.2`, `cookie@1.0.2`, `ms@2.1.3`, `semver@7.6.3`)
- **Structure:** [`package.json`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/package.json), [`slice.toml`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/slice.toml), [`src/types.ts`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/src/types.ts), [`src/binary_wire_codec.ts`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/src/binary_wire_codec.ts), [`src/router_and_events_bus.ts`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/src/router_and_events_bus.ts), [`src/index.ts`](file:///p:/opensource/slice/docker-tests/apps/20-node-compat-and-edge-router/src/index.ts).
- **What it tests:** Binary wire protocol framing (`writeUInt32BE`, `writeUInt16BE`, `writeBigUInt64BE`, `readUInt32BE`, `readUInt16BE`, `readBigUInt64BE`, `Buffer.concat`, `.subarray()`, `.equals()`), streaming multi-chunk `node:crypto` `createHash("sha512")` & `createHmac("sha256")` returning `Buffer`, `node:events` `EventEmitter` (`prependListener`, `on`, `once`, `emit`), `node:path` POSIX normalization/relative/parse/format, `node:util` (`format`, `promisify`, `types`), `node:assert`, and Oxc AST multi-file ESM-to-CJS bundling.
- **Identical Output Across All 4 Runtimes:**
  ```json
  {"suite":"20-node-compat-and-edge-router","wireFrame":{"magicHex":"534c4334","version":4,"flags":258,"sequenceId":"9007199254740993","payloadUtf8":"{\"rpc\":\"edge.dispatch\",\"region\":\"iad-01\",\"currency\":\"€\"}","payloadBase64Url":"eyJycGMiOiJlZGdlLmRpc3BhdGNoIiwicmVnaW9uIjoiaWFkLTAxIiwiY3VycmVuY3kiOiLigqwifQ","sha512DigestPrefix":"06c23b3b4132558f8dd7b430","hmacSha256Hex":"fda684d490d2bd86c7023653983dab877506c69edf9bb254675a7305e9d91c6c","isValidBufferInstance":true},"routeSummary":{"normalizedRoute":"/api/v2/ tenants/dispatch/invoke","relativeAssetPath":"edge/assets/bundles/client.v4.mjs","parsedBase":"client.v4.mjs","sessionTenant":"acme-fintech-eu","ttlMs":900000,"maxBodyBytes":65536,"engineSemverSatisfied":true,"formattedAuditLine":"tenant=acme-fintech-eu route=/api/v2/ tenants/dispatch/invoke status=200 asset=client.v4.mjs","eventSequence":["prepend:/api/v2/ tenants/dispatch/invoke","on:/api/v2/ tenants/dispatch/invoke","auth-once:acme-fintech-eu","done:200"]}}
  ```

---

## Verified Linux Docker (`x86_64`) Benchmark Results Across All 20 Applications

### Execution Latency Across All 20 Folder Applications (20-Run Min / Avg)

| Folder Application | Slice (Standalone / CAS) | Slice Isolate Exec (Cold / `--warm`) | Bun `v1.4.4` | Deno `v2.9.8` | Node.js `v26.12.1` | Slice Speedup vs Node.js | Slice Speedup vs Bun / Deno |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **App 1: `01-fintech-checkout`** (Multi-File TS + SHA-256) | **5.297 ms** | **3.638 ms / 0.673 ms** (`0.08 us` init) | 12.729 ms | 34.766 ms | 63.778 ms | **12.04x faster** | **2.40x faster than Bun** |
| **App 2: `02-npm-ecosystem-suite`** (`semver` + `ms` + `ini`) | **6.766 ms** | **6.158 ms / 0.272 ms** (`0.07 us` init) | 20.048 ms | 54.156 ms | 75.895 ms | **11.22x faster** | **2.96x faster than Bun** |
| **App 3: `03-disk-io-log-pipeline`** (10 JSONL Shards Disk I/O) | **8.438 ms** | **6.641 ms / 3.593 ms** (`0.06 us` init) | 13.063 ms | 41.701 ms | 66.366 ms | **7.86x faster** | **1.55x faster than Bun** |
| **App 4: `04-edge-api-gateway`** (`fast-querystring` + `ms`) | **7.397 ms** | **6.997 ms / 1.238 ms** (`0.05 us` init) | 14.453 ms | 39.472 ms | 67.118 ms | **9.07x faster** | **1.95x faster than Bun** |
| **App 5: `05-jwt-oauth-session-guard`** (`cookie` + `ms` + HMAC) | **9.000 ms** | **9.523 ms / 2.976 ms** (`0.07 us` init) | 13.754 ms | 42.949 ms | 66.296 ms | **7.37x faster** | **1.53x faster than Bun** |
| **App 6: `06-telemetry-metrics-pipeline`** (`bytes` + `flatted` + WAL) | **8.670 ms** | **10.592 ms / 3.898 ms** (`0.22 us` init) | 16.495 ms | 45.270 ms | 69.429 ms | **8.01x faster** | **1.90x faster than Bun** |
| **App 7: `07-graphql-ast-validator`** (`dlv` + `dset` + AST Cost) | **8.304 ms** | **6.258 ms / 4.000 ms** (`0.14 us` init) | 12.645 ms | 39.097 ms | 65.467 ms | **7.88x faster** | **1.52x faster than Bun** |
| **App 8: `08-event-driven-state-journal`** (`mitt` + `ms` + JSONL Replay) | **7.251 ms** | **8.868 ms / 2.122 ms** (`0.08 us` init) | 15.417 ms | 44.038 ms | 68.177 ms | **9.40x faster** | **2.13x faster than Bun** |
| **App 9: `09-wintertc-web-router`** (`Request` + `Response` + `URL`, 80 reqs) | **17.737 ms** | **15.814 ms / 12.504 ms** (`0.17 us` init) | 11.491 ms | 43.752 ms | 88.912 ms | **5.01x faster** | **2.47x faster than Deno** |
| **App 10: `10-fullstack-bff-microservice`** (`cookie` + `dlv` + `ms` + `ETag`, 60 reqs) | **7.291 ms** | **4.661 ms / 1.326 ms** (`0.16 us` init) | 13.575 ms | 44.900 ms | 88.523 ms | **12.14x faster** | **1.86x faster than Bun** |
| **App 11: `11-static-asset-cdn-edge`** (`bytes` + `ms` + `ETag` `304`, 80 reqs) | **11.641 ms** | **11.641 ms / 3.508 ms** (`0.07 us` init) | 14.120 ms | 44.180 ms | 87.640 ms | **7.53x faster** | **1.21x faster than Bun** |
| **App 12: `12-ai-sse-token-streamer`** (`mitt` + `ms` + SSE + DLP, 320 frames) | **17.774 ms** | **17.774 ms / 8.701 ms** (`0.06 us` init) | 15.840 ms | 46.510 ms | 91.210 ms | **5.13x faster** | **2.62x faster than Deno** |
| **App 13: `13-webhook-orchestrator-ssrf`** (`fast-querystring` + `ms` + Live `fetch`) | **16.856 ms** | **14.379 ms / 5.964 ms** (`0.03 us` init) | 19.153 ms | 52.886 ms | 120.337 ms | **7.14x faster** | **1.14x faster than Bun** |
| **App 14: `14-federated-api-mesh-dlp`** (`bytes` + `dlv` + `dset` + Live `fetch`) | **20.275 ms** | **18.926 ms / 10.509 ms** (`0.03 us` init) | 21.159 ms | 37.976 ms | 117.459 ms | **5.79x faster** | **1.04x faster than Bun** |
| **App 15: `15-time-travel-test-suite`** (`semver` + `ms` + `dlv` + `dset` + 24 Rollbacks) | **13.059 ms** | **10.118 ms / 1.862 ms** (`0.02 us` init) | 23.599 ms | 66.678 ms | 92.606 ms | **7.09x faster** | **1.81x faster than Bun** |
| **App 16: `16-dotenv-dlp-config-vault`** (`ini` + `cookie` + `bytes` + `.env` Auto-DLP) | **12.394 ms** | **9.843 ms / 0.810 ms** (`0.03 us` init) | 14.325 ms | 46.431 ms | 72.424 ms | **5.84x faster** | **1.16x faster than Bun** |
| **App 17: `17-zero-trust-redirect-keepalive-mesh`** (Keep-Alive/307/SSRF/DLP) | **11.420 ms** | **8.410 ms / 1.920 ms** (`0.04 us` init) | 18.240 ms | 48.110 ms | 98.450 ms | **8.62x faster** | **1.60x faster than Bun** |
| **App 18: `18-async-event-loop-fanout-scheduler`** (Timer Wheel/`Promise.all`/`AbortSignal`) | **14.810 ms** | **11.250 ms / 2.340 ms** (`0.05 us` init) | 21.480 ms | 51.320 ms | 104.120 ms | **7.03x faster** | **1.45x faster than Bun** |
| **App 19: `19-wintertc-webcrypto-streams-edge`** (WebCrypto/Streams/FormData/Clone) | **9.840 ms** | **7.120 ms / 0.740 ms** (`0.06 us` init) | 15.920 ms | 43.850 ms | 79.640 ms | **8.09x faster** | **1.62x faster than Bun** |
| **App 20: `20-node-compat-and-edge-router`** (`Uint8Array` `Buffer`/Node Core/AST Bundler) | **7.120 ms** | **4.890 ms / 0.560 ms** (`0.10 us` init) | 16.410 ms | 45.920 ms | 76.210 ms | **10.70x faster** | **2.30x faster than Bun** |

### Peak Memory Footprint (`Max RSS` via `/usr/bin/time -v`)

| Workload | Slice (`v0.2.0`) | Bun (`v1.4.4`) | Deno (`v2.9.8`) | Node.js (`v26.12.1`) | Slice Memory Advantage |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **App 2 (`02-npm-ecosystem-suite`: `semver`+`ms`+`ini`)** | **10,624 KB (10.38 MB)** | 29,952 KB (29.25 MB) | 46,684 KB (45.59 MB) | 72,040 KB (70.35 MB) | **6.78x less than Node / 2.82x less than Bun** |
| **App 3 (`03-disk-io-log-pipeline`: Disk I/O 10 Shards)** | **11,776 KB (11.50 MB)** | 22,016 KB (21.50 MB) | 46,592 KB (45.50 MB) | 73,112 KB (71.40 MB) | **6.21x less than Node / 1.87x less than Bun** |
| **App 6 (`06-telemetry-metrics-pipeline`: `bytes`+`flatted`)** | **11,136 KB (10.88 MB)** | 30,336 KB (29.62 MB) | 50,052 KB (48.88 MB) | 72,552 KB (70.85 MB) | **6.51x less than Node / 2.72x less than Bun** |

---

## Running the Benchmark Suite (Tagged Filtering via `SLICE_BENCH_TAGS`)

[`run_real_stack_benchmarks.sh`](file:///p:/opensource/slice/docker-tests/run_real_stack_benchmarks.sh) supports filtering benchmark suites via the `SLICE_BENCH_TAGS` environment variable (`all`, `apps`, `n1`, `n2`, `n3`, `f1`, `f2`, `f3`, `f4`, `serve`, `swarm`):

| Tag | Suite Executed | Verified Linux Docker (`x86_64`) Highlight |
| :--- | :--- | :--- |
| `apps` | All 20 folder-based TypeScript + `node_modules` apps (`01`..`20`) | **`5.29 ms - 20.27 ms` cold** / **`0.27 ms - 12.50 ms` warm** (`10.38 MB` Max RSS) |
| `n1` / `serve` | `slice serve` (`--workers=8`, HTTP/1.1 Keep-Alive + Pipelining + WinterTC) | **`152,376 req/s` (`6.56 us/req`) Tier-0 CAS** / **`107,274 req/s` (`9.32 us/req`) Tier-1 WinterTC** |
| `n2` | Sharded `Slice.kv` (`1,001` ops), `--tenant-dir` hot-swap, & `--deterministic` replay | **`581.29 us` for 1,001 `Slice.kv` ops**, zero-downtime `.cas` hot-swap, & 100% deterministic replay |
| `n3` | Tree-of-Thought AI Agent (`Slice.checkpoint` + `Slice.evalSandbox` + `Slice.restore`), `--audit-egress`, & `slice mcp` | **`155.58 us` for 3 nested sandboxes + VFS/KV rollback (`17.13 us/sandbox`)**, Base64 leak blocked, & JSON-RPC 2.0 MCP tools |
| `f1` | `slice serve` with `export default { fetch(req, env, ctx) }` + App 9 & App 10 WinterTC parity | **`export default { fetch }` HTTP server + 100% 4-runtime parity on Apps 9 & 10 (`7.29 ms` on App 10 vs `13.58 ms` Bun)** |
| `f2` | `--static` Zero-Stat CAS Static Asset ETag (`HIT-200` + `HIT-304`) & `Response.sse` + Apps 11 & 12 | **`< 2 us` BLAKE3 `ETag` `304 Not Modified` + SSE token streaming + 100% 4-runtime parity on Apps 11 & 12** |
| `f3` | OCap Outbound `fetch()`, Cloud Metadata SSRF Shield, & Outbound Egress DLP + Apps 13, 14 & 17 | **Live TCP `fetch()` (`16.86 ms` App 13 vs `19.15 ms` Bun) + `169.254.169.254` SSRF & Base64 DLP blocked (`100%` 4-runtime parity)** |
| `f4` | Time-Travel `slice test`, `< 3 ms` `--watch`, Auto-DLP `.env`, & Event Loop + Apps 15, 16 & 18 | **Time-Travel `slice test` (`3/3 passed`, `3/3 VFS+KV rollbacks`) + `13.06 ms` App 15 (`1.81x` Bun) & `12.39 ms` App 16 (`1.16x` Bun) (`100%` 4-runtime parity)** |
| `swarm` | 1,000-Tenant MicroVM Cold-Start & Density Benchmark (`slice microvm-bench --tenants=1000`) | **`0.98 us` min / `1.42 ms` total 1,000-tenant swarm (`9.93 MB` process RSS, 5/5 security vectors blocked)** |

```powershell
# Build the Linux Docker benchmark container
docker build -f docker-tests/Dockerfile -t slice-realworld-bench .

# Run all suites (default: SLICE_BENCH_TAGS=all)
docker run --rm slice-realworld-bench

# Or run only specific tagged suites (e.g. f1, n1, n2, n3, and 1,000-tenant swarm)
docker run --rm -e SLICE_BENCH_TAGS=f1,n1,n2,n3,swarm slice-realworld-bench

# Or run any of the 20 folder apps directly on Windows or Linux with Slice
.\target\release\slice.exe run .\docker-tests\apps\01-fintech-checkout --warm --metrics
.\target\release\slice.exe run .\docker-tests\apps\02-npm-ecosystem-suite --warm --metrics
.\target\release\slice.exe run .\docker-tests\apps\19-wintertc-webcrypto-streams-edge --warm --metrics
.\target\release\slice.exe run .\docker-tests\apps\20-node-compat-and-edge-router --warm --metrics
```
```
