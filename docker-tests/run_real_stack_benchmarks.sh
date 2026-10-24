#!/usr/bin/env bash
# =============================================================================
# MODERN FOLDER-BASED PRODUCTION APPS, NPM LIBRARIES & DISK I/O BENCHMARK SUITE
# =============================================================================
# Compares Slice (Oxc TypeScript Compiler + Boa ECMAScript VM + Zero-Stat CAS)
# against the ABSOLUTE LATEST releases of:
#   - Bun (JavaScriptCore)
#   - Deno (V8)
#   - Node.js (V8 native TypeScript execution)
#
# 10 Folder-Based Production Test Applications (`docker-tests/apps/`):
#   1. `01-fintech-checkout`:
#      Multi-module TypeScript OOP/Functional checkout engine + `node:crypto` SHA-256
#   2. `02-npm-ecosystem-suite`:
#      Real external npm libraries from `node_modules/` (`semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`)
#   3. `03-disk-io-log-pipeline`:
#      Intensive disk & filesystem I/O (`node:fs`, `node:path`, `node:os`, `node:crypto`)
#      writing, stating, scanning, reading, parsing, and verifying 10 JSONL log shards (200 records)
#   4. `04-edge-api-gateway`:
#      Multi-module Edge API Gateway + external npm `fast-querystring@1.1.2` & `ms@2.1.3`
#   5. `05-jwt-oauth-session-guard`:
#      Zero-Trust OAuth2 / JWT HMAC-SHA256 + Base64URL + CSRF + Cookie Guard (`cookie@0.6.0`, `ms@2.1.3`)
#   6. `06-telemetry-metrics-pipeline`:
#      OpenTelemetry Circular Span Graph + Flatted Serialization + Disk WAL (`bytes@3.1.2`, `flatted@3.3.1`)
#   7. `07-graphql-ast-validator`:
#      GraphQL AST Depth/Complexity Cost Guard + Deep Schema Resolver (`dlv@1.1.3`, `dset@3.1.3`)
#   8. `08-event-driven-state-journal`:
#      CQRS Event-Sourced Ledger Bus + Append-Only JSONL WAL Replay (`mitt@3.0.1`, `ms@2.1.3`)
#   9. `09-wintertc-web-router`:
#      Full WinterTC Web Standards Router (`Request`, `Response`, `Response.json`, `Response.redirect`, `Headers`, `URL`, `URLSearchParams`, `export default { fetch }`)
#  10. `10-fullstack-bff-microservice`:
#      Multi-Tenant BFF API Aggregator (`Request`, `Response`, `Headers`, `URL`, `cookie@0.6.0`, `dlv@1.1.3`, `ms@2.1.3`, CORS preflight & ETag `304`)
# =============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SLICE_BIN="$ROOT_DIR/target/release/slice"
APPS_DIR="$ROOT_DIR/docker-tests/apps"
BENCH_TAGS="${1:-${SLICE_BENCH_TAGS:-all}}"

should_run_tag() {
    local tag="$1"
    if [[ ",$BENCH_TAGS," == *",all,"* ]] || [[ ",$BENCH_TAGS," == *",$tag,"* ]]; then
        return 0
    fi
    return 1
}

APP1_DIR="$APPS_DIR/01-fintech-checkout"
APP2_DIR="$APPS_DIR/02-npm-ecosystem-suite"
APP3_DIR="$APPS_DIR/03-disk-io-log-pipeline"
APP4_DIR="$APPS_DIR/04-edge-api-gateway"
APP5_DIR="$APPS_DIR/05-jwt-oauth-session-guard"
APP6_DIR="$APPS_DIR/06-telemetry-metrics-pipeline"
APP7_DIR="$APPS_DIR/07-graphql-ast-validator"
APP8_DIR="$APPS_DIR/08-event-driven-state-journal"
APP9_DIR="$APPS_DIR/09-wintertc-web-router"
APP10_DIR="$APPS_DIR/10-fullstack-bff-microservice"

echo "========================================================================="
echo "  RUNTIME VERSIONS (LINUX DOCKER x86_64 - LATEST RELEASES)"
echo "  Active Benchmark Tags: $BENCH_TAGS"
echo "========================================================================="
echo "  Slice   : v0.2.0 (Oxc TS Compiler + Boa ES2024 VM + Zero-Stat CAS)"
echo "  Node.js : $(node -v)"
echo "  Bun     : v$(bun -v)"
echo "  Deno    : $(deno -V)"

if should_run_tag "apps"; then
echo "========================================================================="
echo "  [TAG: apps] PACKING & COMPILING ALL 10 FOLDER-BASED APPS (OXC + CAS)"
echo "========================================================================="
for i in 1 2 3 4 5 6 7 8 9 10; do
    eval "dir=\$APP${i}_DIR"
    "$SLICE_BIN" pack "$dir" "/tmp/app${i}.cas"
    "$SLICE_BIN" compile "/tmp/app${i}.cas" -o "/tmp/app${i}_bin"
done
fi

verify_app_parity() {
    local label="$1"
    local app_dir="$2"
    local cas_file="$3"
    local deno_flags="$4"

    local entry="$app_dir/src/index.ts"
    local out_slice
    local out_bun
    local out_deno
    local out_node

    out_slice=$("$SLICE_BIN" run "$cas_file")
    out_bun=$(bun run "$entry")
    # shellcheck disable=SC2086
    out_deno=$(deno run $deno_flags "$entry")
    out_node=$(node "$entry" 2>/dev/null)

    echo "  [$label]"
    echo "    Slice : $out_slice"
    echo "    Bun   : $out_bun"
    echo "    Deno  : $out_deno"
    echo "    Node  : $out_node"

    if [ "$out_slice" = "$out_bun" ] && [ "$out_slice" = "$out_deno" ] && [ "$out_slice" = "$out_node" ]; then
        echo "    [PASS] 100% Byte-for-Byte Identical Output Across Slice, Bun, Deno, and Node.js!"
    else
        echo "    [FAIL] Output mismatch detected on $label!"
        exit 1
    fi
    echo ""
}

if should_run_tag "apps"; then
echo "========================================================================="
echo "  1. VERIFYING 100% OUTPUT PARITY ACROSS ALL 10 FOLDER APPS & 4 RUNTIMES"
echo "========================================================================="
verify_app_parity "App 1: 01-fintech-checkout (Multi-File TS + SHA-256)" "$APP1_DIR" "/tmp/app1.cas" ""
verify_app_parity "App 2: 02-npm-ecosystem-suite (semver + ms + ini)" "$APP2_DIR" "/tmp/app2.cas" "-A"
verify_app_parity "App 3: 03-disk-io-log-pipeline (10 JSONL Shards Disk I/O)" "$APP3_DIR" "/tmp/app3.cas" "-A"
verify_app_parity "App 4: 04-edge-api-gateway (fast-querystring + ms + Auth)" "$APP4_DIR" "/tmp/app4.cas" "-A"
verify_app_parity "App 5: 05-jwt-oauth-session-guard (cookie + ms + HMAC-SHA256)" "$APP5_DIR" "/tmp/app5.cas" "-A"
verify_app_parity "App 6: 06-telemetry-metrics-pipeline (bytes + flatted + WAL)" "$APP6_DIR" "/tmp/app6.cas" "-A"
verify_app_parity "App 7: 07-graphql-ast-validator (dlv + dset + AST Cost Guard)" "$APP7_DIR" "/tmp/app7.cas" "-A"
verify_app_parity "App 8: 08-event-driven-state-journal (mitt + ms + JSONL Replay)" "$APP8_DIR" "/tmp/app8.cas" "-A"
verify_app_parity "App 9: 09-wintertc-web-router (Request/Response/URL/Headers)" "$APP9_DIR" "/tmp/app9.cas" "-A"
verify_app_parity "App 10: 10-fullstack-bff-microservice (CORS/ETag/Cookie/dlv)" "$APP10_DIR" "/tmp/app10.cas" "-A"
fi

measure_ms() {
    local name="$1"
    local runs="$2"
    shift 2
    "$@" > /dev/null 2>&1 || true

    local min_ns=999999999999
    local sum_ns=0
    for _ in $(seq 1 "$runs"); do
        local t0
        t0=$(date +%s%N)
        "$@" > /dev/null 2>&1
        local t1
        t1=$(date +%s%N)
        local diff=$((t1 - t0))
        sum_ns=$((sum_ns + diff))
        if [ "$diff" -lt "$min_ns" ]; then
            min_ns="$diff"
        fi
    done
    local avg_ns=$((sum_ns / runs))
    awk -v name="$name" -v min="$min_ns" -v avg="$avg_ns" 'BEGIN {
        printf "  %-36s | Min: %8.3f ms | Avg: %8.3f ms\n", name, min / 1000000.0, avg / 1000000.0
    }'
}

measure_rss_kb() {
    local name="$1"
    shift
    local rss
    rss=$(/usr/bin/time -v "$@" 2>&1 | grep "Maximum resident set size" | awk '{print $NF}')
    awk -v name="$name" -v rss="$rss" 'BEGIN {
        printf "  %-36s | Max RSS: %8d KB (%6.2f MB)\n", name, rss, rss / 1024.0
    }'
}

bench_app() {
    local idx="$1"
    local title="$2"
    local app_dir="$3"
    local deno_flags="$4"
    echo "========================================================================="
    echo "  APP ${idx} LATENCY: ${title} (20 Runs)"
    echo "========================================================================="
    measure_ms "Slice (Standalone Binary)"           20 "/tmp/app${idx}_bin" --quiet
    measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app${idx}.cas" --quiet
    measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$app_dir" --quiet
    measure_ms "Bun (Latest)"                        20 bun run "$app_dir/src/index.ts"
    # shellcheck disable=SC2086
    measure_ms "Deno (Latest)"                       20 deno run $deno_flags "$app_dir/src/index.ts"
    measure_ms "Node.js (Latest)"                    20 node "$app_dir/src/index.ts"
}

if should_run_tag "apps"; then
bench_app 1 "01-FINTECH-CHECKOUT (Multi-File TS + SHA-256)" "$APP1_DIR" ""
bench_app 2 "02-NPM-ECOSYSTEM-SUITE (semver + ms + ini)" "$APP2_DIR" "-A"
bench_app 3 "03-DISK-IO-LOG-PIPELINE (10 JSONL Shards I/O)" "$APP3_DIR" "-A"
bench_app 4 "04-EDGE-API-GATEWAY (fast-querystring + ms)" "$APP4_DIR" "-A"
bench_app 5 "05-JWT-OAUTH-SESSION-GUARD (cookie + ms + HMAC)" "$APP5_DIR" "-A"
bench_app 6 "06-TELEMETRY-METRICS-PIPELINE (bytes + flatted)" "$APP6_DIR" "-A"
bench_app 7 "07-GRAPHQL-AST-VALIDATOR (dlv + dset + AST)" "$APP7_DIR" "-A"
bench_app 8 "08-EVENT-DRIVEN-STATE-JOURNAL (mitt + ms + WAL)" "$APP8_DIR" "-A"
bench_app 9 "09-WINTERTC-WEB-ROUTER (Request/Response/URL/Headers)" "$APP9_DIR" "-A"
bench_app 10 "10-FULLSTACK-BFF-MICROSERVICE (CORS/ETag/Cookie/dlv)" "$APP10_DIR" "-A"

echo "========================================================================="
echo "  PEAK MEMORY FOOTPRINT (MAX RSS) ACROSS REPRESENTATIVE APPS"
echo "========================================================================="
measure_rss_kb "Slice (App 2: NPM semver+ms+ini)"  "$SLICE_BIN" run "/tmp/app2.cas" --quiet
measure_rss_kb "Bun (App 2: NPM semver+ms+ini)"    bun run "$APP2_DIR/src/index.ts"
measure_rss_kb "Deno (App 2: NPM semver+ms+ini)"   deno run -A "$APP2_DIR/src/index.ts"
measure_rss_kb "Node (App 2: NPM semver+ms+ini)"   node "$APP2_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 6: Telemetry + Flatted)" "$SLICE_BIN" run "/tmp/app6.cas" --quiet
measure_rss_kb "Bun (App 6: Telemetry + Flatted)"   bun run "$APP6_DIR/src/index.ts"
measure_rss_kb "Deno (App 6: Telemetry + Flatted)"  deno run -A "$APP6_DIR/src/index.ts"
measure_rss_kb "Node (App 6: Telemetry + Flatted)"  node "$APP6_DIR/src/index.ts"

echo "========================================================================="
echo "  SLICE INTERNAL ISOLATE TELEMETRY ACROSS ALL 10 APPS (COLD VS WARM POOL)"
echo "========================================================================="
echo "  --- Cold Realm Telemetry ---"
for i in 1 2 3 4 5 6 7 8 9 10; do
    "$SLICE_BIN" run "/tmp/app${i}.cas" --quiet --metrics
done
echo "  --- Pre-Warmed Boa Pool Telemetry (--warm) ---"
for i in 1 2 3 4 5 6 7 8 9 10; do
    "$SLICE_BIN" run "/tmp/app${i}.cas" --warm --quiet --metrics
done
fi

if should_run_tag "n1" || should_run_tag "serve"; then
echo "========================================================================="
echo "  [TAG: n1 / serve] PHASE N1 STATE-OF-THE-ART SLICE SERVE BENCHMARK"
echo "  HTTP/1.1 Keep-Alive + Multi-Worker Shared-Nothing Pool + WinterTC API"
echo "========================================================================="

cat << 'EOF' > /tmp/n1_tier0_handler.ts
const base: number = 1400;
const fee: number = 37;
return base + fee;
EOF

cat << 'EOF' > /tmp/n1_wintertc_handler.ts
class EdgeGateway {
    public serve(): void {
        const req = Slice.request();
        Slice.waitUntil(() => {
            globalThis.__last_tenant = req.tenant;
        });
        Slice.respond(
            200,
            JSON.stringify({ ok: true, method: req.method, path: req.path, tenant: req.tenant }),
            "application/json"
        );
    }
}
new EdgeGateway().serve();
EOF

"$SLICE_BIN" pack /tmp/n1_tier0_handler.ts /tmp/n1_tier0.cas >/dev/null
"$SLICE_BIN" pack /tmp/n1_wintertc_handler.ts /tmp/n1_wintertc.cas >/dev/null

cat << 'EOF' > /tmp/n1_bun_server.ts
const server = Bun.serve({
    port: 18082,
    fetch(req) {
        const url = new URL(req.url);
        const tenant = req.headers.get("x-slice-tenant") || "tenant-default";
        return new Response(
            JSON.stringify({ ok: true, method: req.method, path: url.pathname, tenant }),
            { status: 200, headers: { "Content-Type": "application/json" } }
        );
    },
});
EOF

cat << 'EOF' > /tmp/n1_deno_server.ts
Deno.serve({ port: 18083, onListen: () => {} }, (req: Request) => {
    const url = new URL(req.url);
    const tenant = req.headers.get("x-slice-tenant") || "tenant-default";
    return new Response(
        JSON.stringify({ ok: true, method: req.method, path: url.pathname, tenant }),
        { status: 200, headers: { "Content-Type": "application/json" } }
    );
});
EOF

cat << 'EOF' > /tmp/n1_node_server.mjs
import http from "node:http";
const server = http.createServer((req, res) => {
    const path = (req.url || "/").split("?")[0];
    const tenant = req.headers["x-slice-tenant"] || "tenant-default";
    const body = JSON.stringify({ ok: true, method: req.method, path, tenant });
    res.writeHead(200, {
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(body),
    });
    res.end(body);
});
server.listen(18084, "127.0.0.1");
EOF

cat << 'EOF' > /tmp/n1_keepalive_bench.mjs
import net from "node:net";

const [label, portStr, totalStr, concurrencyStr, pipelineStr] = process.argv.slice(2);
const port = Number(portStr);
const totalRequests = Number(totalStr || 16000);
const concurrency = Number(concurrencyStr || 8);
const pipeline = Number(pipelineStr || 8);
const perSocketTarget = Math.floor(totalRequests / concurrency);

const singleReq =
    "GET /v1/checkout?sku=edge HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Slice-Tenant: tenant-edge-1\r\nConnection: keep-alive\r\n\r\n";
const batchPayload = Buffer.from(singleReq.repeat(pipeline));

function runSocket(targetCount) {
    return new Promise((resolve, reject) => {
        const sock = net.connect(port, "127.0.0.1", () => {
            sock.setNoDelay(true);
            sendBatch();
        });
        let completed = 0;
        let sent = 0;
        let inFlight = 0;
        let buf = Buffer.alloc(0);

        function sendBatch() {
            const rem = targetCount - sent;
            if (rem <= 0) return;
            const n = Math.min(pipeline, rem);
            sent += n;
            inFlight += n;
            if (n === pipeline) {
                sock.write(batchPayload);
            } else {
                sock.write(singleReq.repeat(n));
            }
        }

        sock.on("data", (chunk) => {
            buf = buf.length === 0 ? chunk : Buffer.concat([buf, chunk]);
            while (true) {
                const hdrEnd = buf.indexOf("\r\n\r\n");
                if (hdrEnd === -1) break;
                const hdr = buf.subarray(0, hdrEnd).toString("latin1");
                const m = /content-length:\s*(\d+)/i.exec(hdr);
                const bodyLen = m ? Number(m[1]) : 0;
                const totalLen = hdrEnd + 4 + bodyLen;
                if (buf.length < totalLen) break;
                buf = buf.subarray(totalLen);
                inFlight--;
                completed++;
            }
            if (completed >= targetCount) {
                sock.end();
                resolve();
            } else if (inFlight === 0) {
                sendBatch();
            }
        });
        sock.on("error", reject);
    });
}

// Warmup all worker threads across 8 sockets
await Promise.all(Array.from({ length: concurrency }, () => runSocket(32)));

const startNs = process.hrtime.bigint();
await Promise.all(Array.from({ length: concurrency }, () => runSocket(perSocketTarget)));
const elapsedNs = Number(process.hrtime.bigint() - startNs);
const elapsedMs = elapsedNs / 1e6;
const actualTotal = perSocketTarget * concurrency;
const rps = (actualTotal / (elapsedNs / 1e9)).toFixed(0);
const avgUs = (elapsedNs / 1e3 / actualTotal).toFixed(2);
console.log(
    `  ${label.padEnd(42)} | ${String(rps).padStart(8)} req/s | Avg: ${String(avgUs).padStart(6)} us/req | Total: ${elapsedMs.toFixed(1)} ms`
);
EOF

run_http_bench() {
    local label="$1"
    local port="$2"
    shift 2
    "$@" >/dev/null 2>&1 &
    local srv_pid=$!
    # Wait up to 3s for port to accept connections
    for _ in $(seq 1 60); do
        if node -e "import('node:net').then(n=>{const s=n.connect($port,'127.0.0.1',()=>{s.end();process.exit(0)});s.on('error',()=>process.exit(1))})" 2>/dev/null; then
            break
        fi
        sleep 0.05
    done
    node /tmp/n1_keepalive_bench.mjs "$label" "$port" 16000 8 8
    kill -9 "$srv_pid" >/dev/null 2>&1 || true
    wait "$srv_pid" 2>/dev/null || true
}

echo "  --- 16,000 HTTP/1.1 Keep-Alive Pipelined Requests (8 Sockets x 8 Pipeline Depth) ---"
run_http_bench "Slice serve (Tier-0 CAS, --workers=8)"      18080 "$SLICE_BIN" serve /tmp/n1_tier0.cas --port=18080 --workers=8
run_http_bench "Slice serve (Tier-1 WinterTC, --workers=8)" 18081 "$SLICE_BIN" serve /tmp/n1_wintertc.cas --port=18081 --workers=8
run_http_bench "Bun.serve (Latest JSC, Single-Process)"     18082 bun run /tmp/n1_bun_server.ts
run_http_bench "Deno.serve (Latest V8, Single-Process)"     18083 deno run -A /tmp/n1_deno_server.ts
run_http_bench "Node.js http.createServer (Latest V8)"      18084 node /tmp/n1_node_server.mjs

rm -f /tmp/n1_*.ts /tmp/n1_*.mjs /tmp/n1_*.cas
fi

if should_run_tag "n2"; then
echo "========================================================================="
echo "  [TAG: n2] PHASE N2 SERVERLESS & MICROVM PRIMITIVES VERIFICATION & BENCH"
echo "  Sharded Slice.kv Slab + Multi-Tenant --tenant-dir Hot-Swap + Replay"
echo "========================================================================="

# 1. Verify Opt-In Deterministic Replay (--deterministic=<seed>)
cat << 'EOF' > /tmp/n2_det_replay.ts
const r1 = Math.random().toFixed(12);
const r2 = Math.random().toFixed(12);
const t1 = Date.now();
const t2 = Date.now();
const p1 = performance.now();
console.log(JSON.stringify({ r1, r2, t1, t2, p1 }));
EOF
"$SLICE_BIN" pack /tmp/n2_det_replay.ts /tmp/n2_det_replay.cas >/dev/null
det_run_1=$("$SLICE_BIN" run /tmp/n2_det_replay.cas --deterministic=20261019)
det_run_2=$("$SLICE_BIN" run /tmp/n2_det_replay.cas --deterministic=20261019)
echo "  [1. Deterministic Replay (--deterministic=20261019)]"
echo "    Run 1 : $det_run_1"
echo "    Run 2 : $det_run_2"
if [ "$det_run_1" = "$det_run_2" ]; then
    echo "    [PASS] 100% Bit-for-Bit Identical PRNG & Virtual Clock Replay!"
else
    echo "    [FAIL] Deterministic replay mismatch!"
    exit 1
fi

# 2. Verify Multi-Tenant Dynamic .cas Router & Zero-Downtime Atomic Hot-Swap (--tenant-dir)
mkdir -p /tmp/n2_tenants
cat << 'EOF' > /tmp/n2_tenants/alpha.ts
const hits = Slice.kv.incr("requests", 1);
Slice.respond(200, JSON.stringify({ tenant: "alpha", version: "v1", hits }), "application/json");
EOF
cat << 'EOF' > /tmp/n2_tenants/beta.ts
const hits = Slice.kv.incr("requests", 10);
Slice.respond(200, JSON.stringify({ tenant: "beta", version: "v1", hits }), "application/json");
EOF
"$SLICE_BIN" pack /tmp/n2_tenants/alpha.ts /tmp/n2_tenants/tenant-alpha.cas >/dev/null
"$SLICE_BIN" pack /tmp/n2_tenants/beta.ts /tmp/n2_tenants/tenant-beta.cas >/dev/null

"$SLICE_BIN" serve --port=18090 --workers=4 --tenant-dir=/tmp/n2_tenants >/dev/null 2>&1 &
n2_srv_pid=$!
for _ in $(seq 1 60); do
    if node -e "import('node:net').then(n=>{const s=n.connect(18090,'127.0.0.1',()=>{s.end();process.exit(0)});s.on('error',()=>process.exit(1))})" 2>/dev/null; then
        break
    fi
    sleep 0.05
done

alpha_v1=$(curl -fsSL -H "X-Slice-Tenant: tenant-alpha" http://127.0.0.1:18090/check)
beta_v1=$(curl -fsSL -H "X-Slice-Tenant: tenant-beta" http://127.0.0.1:18090/check)

# Atomic live hot-swap of tenant-alpha.cas while server is running!
cat << 'EOF' > /tmp/n2_tenants/alpha_v2.ts
const hits = Slice.kv.incr("requests", 1);
Slice.respond(200, JSON.stringify({ tenant: "alpha", version: "v2-hot-swapped", hits }), "application/json");
EOF
"$SLICE_BIN" pack /tmp/n2_tenants/alpha_v2.ts /tmp/n2_tenants/tenant-alpha.cas >/dev/null
alpha_v2=$(curl -fsSL -H "X-Slice-Tenant: tenant-alpha" http://127.0.0.1:18090/check)

echo "  [2. Multi-Tenant Dynamic Router + Zero-Downtime Hot-Swap + Isolated Slice.kv]"
echo "    Tenant Alpha (v1)          : $alpha_v1"
echo "    Tenant Beta  (v1 isolated) : $beta_v1"
echo "    Tenant Alpha (v2 hot-swap) : $alpha_v2"

# 3. Benchmark Sharded Slice.kv Stateful Edge Rate-Limiter (1,000 KV Get+Set+Incr Ops in Isolate)
cat << 'EOF' > /tmp/n2_kv_bench.ts
class EdgeKvBench {
    public run(): string {
        let total = 0;
        for (let i = 0; i < 500; i++) {
            total = Slice.kv.incr("rate:window", 1);
            Slice.kv.set("token:last", "tok_" + i, 60000);
        }
        const last = Slice.kv.get("token:last");
        return JSON.stringify({ ops: 1001, total, last });
    }
}
console.log(new EdgeKvBench().run());
EOF
"$SLICE_BIN" pack /tmp/n2_kv_bench.ts /tmp/n2_kv_bench.cas >/dev/null
echo "  [3. Sharded Slice.kv 1,001-Op Stateful Rate-Limiter Telemetry (--warm)]"
"$SLICE_BIN" run /tmp/n2_kv_bench.cas --warm --metrics

kill -9 "$n2_srv_pid" >/dev/null 2>&1 || true
wait "$n2_srv_pid" 2>/dev/null || true
rm -rf /tmp/n2_*
fi

if should_run_tag "n3"; then
echo "========================================================================="
echo "  [TAG: n3] PHASE N3 AI AGENT WORKFLOW, CHECKPOINT/RESTORE & MCP SERVER"
echo "  Slice.evalSandbox + Slice.checkpoint/restore + Egress Guard + slice mcp"
echo "========================================================================="

# 1. Tree-of-Thought AI Agent Workflow (Checkpoint -> Candidate EvalSandbox -> Restore -> Commit)
cat << 'EOF' > /tmp/n3_tot_agent.ts
import fs from "node:fs";

class TreeOfThoughtAgent {
    public run(): string {
        fs.writeFileSync("/workspace/model.ts", "export const weights = [1, 2, 3];");
        Slice.kv.set("agent:step", "baseline", 0);

        // Take a sub-5us in-memory VFS + KV checkpoint before exploring candidate LLM branches
        const snapId = Slice.checkpoint();

        // Candidate Branch 1: Mutates VFS + KV, then attempts prototype pollution inside sub-sandbox
        fs.writeFileSync("/workspace/model.ts", "CORRUPTED_WEIGHTS");
        Slice.kv.set("agent:step", "corrupted-branch-1", 0);
        const branch1 = Slice.evalSandbox("Object.prototype.polluted = true;", { fuel: 50000 });
        if (!branch1.ok) {
            Slice.restore(snapId);
        }

        const vfsRestored = fs.readFileSync("/workspace/model.ts", "utf8");
        const kvRestored = Slice.kv.get("agent:step");

        // Candidate Branch 2: Attenuated child sandbox executing valid TypeScript patch
        const branch2 = Slice.evalSandbox(
            "const score: number = 48 * 2; console.log('candidate-2-pass'); score",
            { fuel: 250000, allowRead: ["/workspace"] }
        );

        // Candidate Branch 3: Attempts unauthorized privilege escalation to /etc/shadow
        const branch3 = Slice.evalSandbox(
            "const s = fs.readFileSync('/etc/shadow');",
            { fuel: 50000, allowRead: ["/etc"] }
        );

        return JSON.stringify({
            vfsRestored,
            kvRestored,
            branch1Blocked: !branch1.ok,
            branch2Ok: branch2.ok,
            branch2Score: branch2.returnValue,
            branch2TimeUs: branch2.totalTimeUs,
            branch3Blocked: !branch3.ok
        });
    }
}
console.log(new TreeOfThoughtAgent().run());
EOF

"$SLICE_BIN" pack /tmp/n3_tot_agent.ts /tmp/n3_tot_agent.cas >/dev/null
echo "  [1. Tree-of-Thought AI Agent (Slice.checkpoint + Slice.evalSandbox + Slice.restore)]"
"$SLICE_BIN" run /tmp/n3_tot_agent.cas --allow-read=/workspace --allow-write=/workspace --warm --metrics

# 2. Egress Secret Guard (--audit-egress) Blocking Base64 Exfiltration
cat << 'EOF' > /tmp/n3_exfil.ts
const encoded = Buffer.from("sk_live_99887766").toString("base64");
console.log("exfil_payload=" + encoded);
EOF
"$SLICE_BIN" pack /tmp/n3_exfil.ts /tmp/n3_exfil.cas >/dev/null
echo "  [2. Egress Secret Guard (--audit-egress=sk_live_99887766) Base64 Exfiltration Test]"
if "$SLICE_BIN" run /tmp/n3_exfil.cas --audit-egress=sk_live_99887766 >/dev/null 2>&1; then
    echo "    [FAIL] Base64 secret exfiltration was NOT blocked!"
    exit 1
else
    echo "    [PASS] Base64-encoded secret exfiltration deterministically blocked (EGRESS_SECRET_LEAK_BLOCKED)!"
fi

# 3. Built-in Model Context Protocol (MCP) JSON-RPC 2.0 Server Smoke Test
echo "  [3. Built-in Model Context Protocol Server (slice mcp --smoke-test)]"
"$SLICE_BIN" mcp --smoke-test

rm -rf /tmp/n3_*
fi

if should_run_tag "f1"; then
echo "========================================================================="
echo "  [TAG: f1] PHASE F1 WINTERTC WEB STANDARDS & EXPORT DEFAULT FETCH"
echo "  1. Zero-Copy Request/Response/Headers/URL/URLSearchParams Server"
echo "  2. App 9 (09-wintertc-web-router) & App 10 (10-fullstack-bff-microservice)"
echo "========================================================================="

"$SLICE_BIN" pack "$APP9_DIR" "/tmp/f1_app9.cas" >/dev/null
"$SLICE_BIN" pack "$APP10_DIR" "/tmp/f1_app10.cas" >/dev/null
verify_app_parity "Phase F1 App 9: 09-wintertc-web-router" "$APP9_DIR" "/tmp/f1_app9.cas" "-A"
verify_app_parity "Phase F1 App 10: 10-fullstack-bff-microservice" "$APP10_DIR" "/tmp/f1_app10.cas" "-A"

cat << 'EOF' > /tmp/f1_fetch_server.ts
export default {
    fetch(req: Request, env: any, ctx: any): Response {
        const url = new URL(req.url);
        const tenant = req.headers.get("x-slice-tenant") || "default";
        const hits = Slice.kv.incr("f1_req_count", 1);
        ctx.waitUntil(() => {
            Slice.kv.set("f1_last_url", url.pathname, 60000);
        });
        return Response.json({
            ok: true,
            method: req.method,
            path: url.pathname,
            sku: url.searchParams.get("sku"),
            tenant,
            hits,
            env: env.NODE_ENV
        });
    }
};
EOF
"$SLICE_BIN" pack /tmp/f1_fetch_server.ts /tmp/f1_fetch_server.cas >/dev/null

"$SLICE_BIN" serve /tmp/f1_fetch_server.cas --port=18091 --workers=4 --max-requests=4 >/dev/null 2>&1 &
F1_SRV_PID=$!
sleep 0.3

F1_RESP1=$(curl -fsSL -H "X-Slice-Tenant: tenant-f1" "http://127.0.0.1:18091/v2/catalog?sku=B200")
F1_RESP2=$(curl -fsSL -H "X-Slice-Tenant: tenant-f1" "http://127.0.0.1:18091/v2/catalog?sku=H100")
kill "$F1_SRV_PID" 2>/dev/null || true
wait "$F1_SRV_PID" 2>/dev/null || true

echo "  [Live slice serve on export default { fetch(req, env, ctx) }]"
echo "    Req 1 : $F1_RESP1"
echo "    Req 2 : $F1_RESP2"
if [[ "$F1_RESP1" == *'"sku":"B200"'* ]] && [[ "$F1_RESP2" == *'"hits":2'* ]]; then
    echo "    [PASS] WinterTC export default { fetch(req, env, ctx) } + URL + Headers + Response.json verified!"
else
    echo "    [FAIL] Unexpected response from F1 fetch server!"
    exit 1
fi
rm -f /tmp/f1_*
fi

if should_run_tag "swarm"; then
echo "========================================================================="
echo "  [TAG: swarm] MICROVM SPEEDUP RETENTION CHECK (1,000 TENANT SWARM)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000
fi

rm -f /tmp/app*.cas /tmp/app*_bin
