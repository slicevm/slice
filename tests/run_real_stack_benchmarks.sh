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
# 24 Folder-Based Production Test Applications (`tests/apps/`) & Framework Suite (`tests/frameworks/`):
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
APPS_DIR="$ROOT_DIR/tests/apps"
FRAMEWORKS_DIR="$ROOT_DIR/tests/frameworks"
CASE_STUDIES_DIR="$ROOT_DIR/tests/case_studies"
BENCH_TAGS="${1:-${SLICE_BENCH_TAGS:-all}}"
METRICS_JSONL="${SLICE_BENCH_JSONL:-/tmp/slice_bench_metrics.jsonl}"
rm -f "$METRICS_JSONL"

SLICE_VERSION="${SLICE_VERSION:-0.10.4}"
SLICE_GIT_COMMIT="${SLICE_GIT_COMMIT:-$(git -C "$ROOT_DIR" rev-parse --short HEAD 2>/dev/null || echo "abdbf89")}"
SLICE_BENCH_TS_UTC="${SLICE_BENCH_TS_UTC:-$(date -u +"%Y-%m-%dT%H:%M:%SZ")}"
CURRENT_APP_IDX=""
CURRENT_BENCH_MODE=""

record_jsonl() {
    echo "$1" >> "$METRICS_JSONL"
}

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
APP11_DIR="$APPS_DIR/11-static-asset-cdn-edge"
APP12_DIR="$APPS_DIR/12-ai-sse-token-streamer"
APP13_DIR="$APPS_DIR/13-webhook-orchestrator-ssrf"
APP14_DIR="$APPS_DIR/14-federated-api-mesh-dlp"
APP15_DIR="$APPS_DIR/15-time-travel-test-suite"
APP16_DIR="$APPS_DIR/16-dotenv-dlp-config-vault"
APP17_DIR="$APPS_DIR/17-zero-trust-redirect-keepalive-mesh"
APP18_DIR="$APPS_DIR/18-async-event-loop-fanout-scheduler"
APP19_DIR="$APPS_DIR/19-wintertc-webcrypto-streams-edge"
APP20_DIR="$APPS_DIR/20-node-compat-and-edge-router"
APP21_DIR="$APPS_DIR/21-websocket-realtime-and-security-guard"
APP22_DIR="$APPS_DIR/22-worker-threads-and-raw-sockets"
APP23_DIR="$APPS_DIR/23-wasm-crypto-and-simd-kernel"
APP24_DIR="$APPS_DIR/24-tsx-ssr-and-wasm-edge-pipeline"
APP25_DIR="$FRAMEWORKS_DIR/01-hono-edge-microservice"
APP26_DIR="$FRAMEWORKS_DIR/02-itty-router-unjs-nitro-stack"
APP32_DIR="$FRAMEWORKS_DIR/03-node-http-express-compat"
APP27_DIR="$CASE_STUDIES_DIR/01-tot-autonomous-sre-agent"
APP28_DIR="$CASE_STUDIES_DIR/02-mcp-code-mode-orchestrator"
APP29_DIR="$CASE_STUDIES_DIR/03-multi-tenant-saas-plugin-mesh"
APP30_DIR="$CASE_STUDIES_DIR/04-wasm-zero-trust-cdr-pipeline"
APP31_DIR="$CASE_STUDIES_DIR/05-time-travel-chaos-fuzzer-payments"
WORKBENCH_DIR="$CASE_STUDIES_DIR/workbench"

UPSTREAM_F3_PID=""
UPSTREAM_WC_PID=""
cleanup_upstream_f3() {
    if [ -n "$UPSTREAM_F3_PID" ]; then
        kill "$UPSTREAM_F3_PID" 2>/dev/null || true
        wait "$UPSTREAM_F3_PID" 2>/dev/null || true
    fi
    if [ -n "$UPSTREAM_WC_PID" ]; then
        kill "$UPSTREAM_WC_PID" 2>/dev/null || true
        wait "$UPSTREAM_WC_PID" 2>/dev/null || true
    fi
    rm -f /tmp/f3_upstream_* /tmp/wc_upstream_*
}
trap cleanup_upstream_f3 EXIT

if should_run_tag "apps" || should_run_tag "f3" || should_run_tag "app22"; then
cat << 'EOF' > /tmp/f3_upstream_server.ts
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if ((req.headers.get("upgrade") || "").toLowerCase() === "websocket") {
            return Slice.upgradeWebSocket({
                open(ws: any) {
                    ws.send(JSON.stringify({ type: "handshake_ack", path: url.pathname }));
                },
                message(ws: any, msg: any) {
                    if (typeof msg === "string") {
                        const parsed = JSON.parse(msg);
                        ws.send(JSON.stringify({
                            type: "telemetry_ack",
                            seq: parsed.seq,
                            tenant: parsed.tenant,
                            digest: parsed.digest,
                            ok: true
                        }));
                    } else if (msg instanceof Uint8Array) {
                        const out = new Uint8Array(msg.length);
                        for (let i = 0; i < msg.length; i++) {
                            out[i] = (msg[i] ^ 0x5a) & 0xff;
                        }
                        ws.send(out);
                    }
                }
            });
        }
        if (url.pathname === "/upstream/webhook") {
            const body = req.jsonSync() as { eventId?: string };
            return Response.json({
                ackId: "ack-" + (body.eventId || "0"),
                status: "accepted"
            });
        }
        if (url.pathname === "/upstream/subgraph/account") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.json({
                accountId,
                tier: "enterprise",
                balanceCents: 14500,
                region: "us-east-1"
            });
        }
        if (url.pathname === "/upstream/redirect-307") {
            return Response.redirect("/upstream/subgraph/account?" + url.searchParams.toString(), 307);
        }
        if (url.pathname === "/upstream/redirect-302") {
            const accountId = url.searchParams.get("account") || "acct-0";
            return Response.redirect("https://partner-gateway.example.com/upstream/subgraph/account?account=" + accountId, 302);
        }
        return Response.json({ ok: true });
    }
};
EOF
"$SLICE_BIN" pack /tmp/f3_upstream_server.ts /tmp/f3_upstream_server.cas >/dev/null
"$SLICE_BIN" serve /tmp/f3_upstream_server.cas --port=18095 --workers=4 >/dev/null 2>&1 &
UPSTREAM_F3_PID=$!

cat << 'EOF' > /tmp/wc_upstream_sockets.mjs
import net from "node:net";
import dgram from "node:dgram";

const tcpSrv = net.createServer((sock) => {
    let buf = "";
    sock.on("data", (chunk) => {
        buf += chunk.toString("utf8");
        if (buf.includes("QUIT")) {
            sock.write("+PONG\r\n+OK\r\n$13\r\nactive-wave-c\r\n+BYE\r\n");
        }
    });
    sock.on("error", () => {});
});
tcpSrv.listen(18096, "127.0.0.1");

const udpSrv = dgram.createSocket("udp4");
udpSrv.on("message", (msg, rinfo) => {
    const text = msg.toString("utf8");
    if (text.startsWith("SLICE_UDP_PROBE:")) {
        udpSrv.send(Buffer.from("UDP_ACK:acme-realtime:seq=1:ok", "utf8"), rinfo.port, rinfo.address);
    }
});
udpSrv.bind(18097, "127.0.0.1");
EOF
node /tmp/wc_upstream_sockets.mjs >/dev/null 2>&1 &
UPSTREAM_WC_PID=$!
sleep 0.3
fi

echo "========================================================================="
echo "  RUNTIME VERSIONS (LINUX DOCKER x86_64 - LATEST RELEASES)"
echo "  Active Benchmark Tags: $BENCH_TAGS"
echo "========================================================================="
echo "  Slice   : v0.10.4 (Oxc TS Compiler + Boa ES2024 VM + Zero-Stat CAS)"
echo "  Node.js : $(node -v)"
echo "  Bun     : v$(bun -v)"
echo "  Deno    : $(deno -V)"

NODE_VER="$(node -v)"
BUN_VER="v$(bun -v)"
DENO_VER="$(deno -V | awk '{print "v"$2}')"
record_jsonl "{\"type\":\"meta\",\"version\":\"$SLICE_VERSION\",\"commit\":\"$SLICE_GIT_COMMIT\",\"timestampUtc\":\"$SLICE_BENCH_TS_UTC\",\"bunVersion\":\"$BUN_VER\",\"denoVersion\":\"$DENO_VER\",\"nodeVersion\":\"$NODE_VER\"}"

if should_run_tag "apps"; then
echo "========================================================================="
echo "  [TAG: apps] PACKING & COMPILING ALL 27 APPS & FRAMEWORK SUITES (OXC + CAS)"
echo "========================================================================="
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 32; do
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
verify_app_parity "App 11: 11-static-asset-cdn-edge (Static ETag/304/bytes/ms)" "$APP11_DIR" "/tmp/app11.cas" "-A"
verify_app_parity "App 12: 12-ai-sse-token-streamer (SSE/ReadableStream/mitt/DLP)" "$APP12_DIR" "/tmp/app12.cas" "-A"
verify_app_parity "App 13: 13-webhook-orchestrator-ssrf (Outbound fetch/SSRF/HMAC)" "$APP13_DIR" "/tmp/app13.cas" "-A"
verify_app_parity "App 14: 14-federated-api-mesh-dlp (Outbound fetch/dlv/dset/DLP)" "$APP14_DIR" "/tmp/app14.cas" "-A"
verify_app_parity "App 15: 15-time-travel-test-suite (Time-Travel VFS+KV/semver/dlv)" "$APP15_DIR" "/tmp/app15.cas" "-A"
verify_app_parity "App 16: 16-dotenv-dlp-config-vault (Auto-DLP .env/ini/cookie/bytes)" "$APP16_DIR" "/tmp/app16.cas" "-A"
verify_app_parity "App 17: 17-zero-trust-redirect-keepalive-mesh (Keep-Alive/307/SSRF/DLP)" "$APP17_DIR" "/tmp/app17.cas" "-A"
verify_app_parity "App 18: 18-async-event-loop-fanout-scheduler (Timers/Microtasks/Fan-Out)" "$APP18_DIR" "/tmp/app18.cas" "-A"
verify_app_parity "App 19: 19-wintertc-webcrypto-streams-edge (WebCrypto/Streams/FormData/Clone)" "$APP19_DIR" "/tmp/app19.cas" "-A"
verify_app_parity "App 20: 20-node-compat-and-edge-router (Uint8Array Buffer/Node Core/AST Bundler)" "$APP20_DIR" "/tmp/app20.cas" "-A"
verify_app_parity "App 21: 21-websocket-realtime-and-security-guard (RFC 6455 WS + N-Sec Hardening)" "$APP21_DIR" "/tmp/app21.cas" "-A"
verify_app_parity "App 22: 22-worker-threads-and-raw-sockets (Worker Threads + TCP/UDP + 5-Ring Sec)" "$APP22_DIR" "/tmp/app22.cas" "-A"
verify_app_parity "App 23: 23-wasm-crypto-and-simd-kernel (Pure-Rust WebAssembly + 5-Ring WASM Sec)" "$APP23_DIR" "/tmp/app23.cas" "-A"
verify_app_parity "App 24: 24-tsx-ssr-and-wasm-edge-pipeline (WebAssembly SLA Scorer + SSR Pipeline)" "$APP24_DIR" "/tmp/app24.cas" "-A"
verify_app_parity "Framework 1 (App 25): 01-hono-edge-microservice (Hono v4.6.20 + hono/cookie + cookie + ms)" "$APP25_DIR" "/tmp/app25.cas" "-A"
verify_app_parity "Framework 2 (App 26): 02-itty-router-unjs-nitro-stack (itty-router v5 + ufo + destr + ohash + radix3)" "$APP26_DIR" "/tmp/app26.cas" "-A"
verify_app_parity "Framework 3 (App 32): 03-node-http-express-compat (node:http + node:stream Express/Polka)" "$APP32_DIR" "/tmp/app32.cas" "-A"
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
    if [ -n "$CURRENT_APP_IDX" ] && [ -n "$CURRENT_BENCH_MODE" ]; then
        local min_ms
        local avg_ms
        min_ms=$(awk -v min="$min_ns" 'BEGIN { printf "%.3f", min / 1000000.0 }')
        avg_ms=$(awk -v avg="$avg_ns" 'BEGIN { printf "%.3f", avg / 1000000.0 }')
        record_jsonl "{\"type\":\"latency\",\"appIdx\":$CURRENT_APP_IDX,\"mode\":\"$CURRENT_BENCH_MODE\",\"minMs\":$min_ms,\"avgMs\":$avg_ms}"
    fi
}

measure_rss_kb() {
    local name="$1"
    shift
    local rss
    rss=$(/usr/bin/time -v "$@" 2>&1 | grep "Maximum resident set size" | awk '{print $NF}')
    awk -v name="$name" -v rss="$rss" 'BEGIN {
        printf "  %-36s | Max RSS: %8d KB (%6.2f MB)\n", name, rss, rss / 1024.0
    }'
    local runtime_key="other"
    if [[ "$name" == Slice* ]]; then runtime_key="slice";
    elif [[ "$name" == Bun* ]]; then runtime_key="bun";
    elif [[ "$name" == Deno* ]]; then runtime_key="deno";
    elif [[ "$name" == Node* ]]; then runtime_key="node";
    fi
    local suite_name
    suite_name=$(echo "$name" | sed -E 's/^(Slice|Bun|Deno|Node)[[:space:]]*\((.*)\)$/\2/')
    record_jsonl "{\"type\":\"rss\",\"suite\":\"$suite_name\",\"runtime\":\"$runtime_key\",\"kb\":$rss}"
}

record_warm_metrics() {
    local app_idx="$1"
    local cas_path="$2"
    local out
    out=$("$SLICE_BIN" run "$cas_path" --warm --quiet --metrics)
    echo "$out"
    local total_us
    total_us=$(echo "$out" | awk -F'Total: ' '{print $2}' | awk '{print $1}')
    if [ -n "$total_us" ]; then
        local total_ms
        total_ms=$(awk -v us="$total_us" 'BEGIN { printf "%.3f", us / 1000.0 }')
        record_jsonl "{\"type\":\"warm_metrics\",\"appIdx\":$app_idx,\"totalMs\":$total_ms}"
    fi
}

bench_app() {
    local idx="$1"
    local title="$2"
    local app_dir="$3"
    local deno_flags="$4"
    if [ -n "${SLICE_ONLY_APPS:-}" ] && [[ ",${SLICE_ONLY_APPS}," != *",${idx},"* ]]; then
        return 0
    fi
    echo "========================================================================="
    echo "  APP ${idx} LATENCY: ${title} (20 Runs)"
    echo "========================================================================="
    CURRENT_APP_IDX="$idx"
    CURRENT_BENCH_MODE="slice_bin"
    measure_ms "Slice (Standalone Binary)"           20 "/tmp/app${idx}_bin" --quiet
    CURRENT_BENCH_MODE="slice_cas"
    measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app${idx}.cas" --quiet
    CURRENT_BENCH_MODE="slice_src"
    measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$app_dir" --quiet
    CURRENT_BENCH_MODE="bun"
    measure_ms "Bun (Latest)"                        20 bun run "$app_dir/src/index.ts"
    CURRENT_BENCH_MODE="deno"
    # shellcheck disable=SC2086
    measure_ms "Deno (Latest)"                       20 deno run $deno_flags "$app_dir/src/index.ts"
    CURRENT_BENCH_MODE="node"
    measure_ms "Node.js (Latest)"                    20 node "$app_dir/src/index.ts"
    CURRENT_APP_IDX=""
    CURRENT_BENCH_MODE=""
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
bench_app 11 "11-STATIC-ASSET-CDN-EDGE (Static ETag/304/bytes/ms)" "$APP11_DIR" "-A"
bench_app 12 "12-AI-SSE-TOKEN-STREAMER (SSE/ReadableStream/mitt/DLP)" "$APP12_DIR" "-A"
bench_app 13 "13-WEBHOOK-ORCHESTRATOR-SSRF (Outbound fetch/SSRF/HMAC)" "$APP13_DIR" "-A"
bench_app 14 "14-FEDERATED-API-MESH-DLP (Outbound fetch/dlv/dset/DLP)" "$APP14_DIR" "-A"
bench_app 15 "15-TIME-TRAVEL-TEST-SUITE (Time-Travel VFS+KV/semver/dlv)" "$APP15_DIR" "-A"
bench_app 16 "16-DOTENV-DLP-CONFIG-VAULT (Auto-DLP .env/ini/cookie/bytes)" "$APP16_DIR" "-A"
bench_app 17 "17-ZERO-TRUST-REDIRECT-KEEPALIVE-MESH (Keep-Alive/307/SSRF/DLP)" "$APP17_DIR" "-A"
bench_app 18 "18-ASYNC-EVENT-LOOP-FANOUT-SCHEDULER (Timers/Microtasks/Fan-Out)" "$APP18_DIR" "-A"
bench_app 19 "19-WINTERTC-WEBCRYPTO-STREAMS-EDGE (WebCrypto/Streams/FormData/Clone)" "$APP19_DIR" "-A"
bench_app 20 "20-NODE-COMPAT-AND-EDGE-ROUTER (Uint8Array Buffer/Node Core/AST Bundler)" "$APP20_DIR" "-A"
bench_app 21 "21-WEBSOCKET-REALTIME-AND-SECURITY-GUARD (RFC 6455 WS + N-Sec Hardening)" "$APP21_DIR" "-A"
bench_app 22 "22-WORKER-THREADS-AND-RAW-SOCKETS (Worker Threads + TCP/UDP + 5-Ring Sec)" "$APP22_DIR" "-A"
bench_app 23 "23-WASM-CRYPTO-AND-SIMD-KERNEL (Pure-Rust WebAssembly + 5-Ring WASM Sec)" "$APP23_DIR" "-A"
bench_app 24 "24-TSX-SSR-AND-WASM-EDGE-PIPELINE (WebAssembly SLA Scorer + SSR Pipeline)" "$APP24_DIR" "-A"
bench_app 25 "FRAMEWORK 1: 01-HONO-EDGE-MICROSERVICE (Hono v4.6.20 + hono/cookie + cookie + ms)" "$APP25_DIR" "-A"
bench_app 26 "FRAMEWORK 2: 02-ITTY-ROUTER-UNJS-NITRO-STACK (itty-router v5 + ufo + destr + ohash + radix3)" "$APP26_DIR" "-A"
bench_app 32 "FRAMEWORK 3: 03-NODE-HTTP-EXPRESS-COMPAT (node:http + node:stream Express/Polka)" "$APP32_DIR" "-A"

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
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 21: RFC 6455 WS + Sec)"  "$SLICE_BIN" run "/tmp/app21.cas" --quiet
measure_rss_kb "Bun (App 21: RFC 6455 WS + Sec)"    bun run "$APP21_DIR/src/index.ts"
measure_rss_kb "Deno (App 21: RFC 6455 WS + Sec)"   deno run -A "$APP21_DIR/src/index.ts"
measure_rss_kb "Node (App 21: RFC 6455 WS + Sec)"   node "$APP21_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 22: Workers + Sockets)"  "$SLICE_BIN" run "/tmp/app22.cas" --quiet
measure_rss_kb "Bun (App 22: Workers + Sockets)"    bun run "$APP22_DIR/src/index.ts"
measure_rss_kb "Deno (App 22: Workers + Sockets)"   deno run -A "$APP22_DIR/src/index.ts"
measure_rss_kb "Node (App 22: Workers + Sockets)"   node "$APP22_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 23: Pure-Rust WASM)"     "$SLICE_BIN" run "/tmp/app23.cas" --quiet
measure_rss_kb "Bun (App 23: Pure-Rust WASM)"       bun run "$APP23_DIR/src/index.ts"
measure_rss_kb "Deno (App 23: Pure-Rust WASM)"      deno run -A "$APP23_DIR/src/index.ts"
measure_rss_kb "Node (App 23: Pure-Rust WASM)"      node "$APP23_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (FW 1: Hono v4 Microservice)" "$SLICE_BIN" run "/tmp/app25.cas" --quiet
measure_rss_kb "Bun (FW 1: Hono v4 Microservice)"   bun run "$APP25_DIR/src/index.ts"
measure_rss_kb "Deno (FW 1: Hono v4 Microservice)"  deno run -A "$APP25_DIR/src/index.ts"
measure_rss_kb "Node (FW 1: Hono v4 Microservice)"  node "$APP25_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (FW 2: itty-router + UnJS)"   "$SLICE_BIN" run "/tmp/app26.cas" --quiet
measure_rss_kb "Bun (FW 2: itty-router + UnJS)"     bun run "$APP26_DIR/src/index.ts"
measure_rss_kb "Deno (FW 2: itty-router + UnJS)"    deno run -A "$APP26_DIR/src/index.ts"
measure_rss_kb "Node (FW 2: itty-router + UnJS)"    node "$APP26_DIR/src/index.ts"

echo "========================================================================="
echo "  SLICE INTERNAL ISOLATE TELEMETRY ACROSS ALL 27 APPS (COLD VS WARM POOL)"
echo "========================================================================="
echo "  --- Cold Realm Telemetry ---"
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 32; do
    "$SLICE_BIN" run "/tmp/app${i}.cas" --quiet --metrics
done
echo "  --- Pre-Warmed Boa Pool Telemetry (--warm) ---"
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 32; do
    record_warm_metrics "$i" "/tmp/app${i}.cas"
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

if should_run_tag "f2"; then
echo "========================================================================="
echo "  [TAG: f2] PHASE F2 ZERO-STAT CAS STATIC ASSETS & AI SSE TOKEN STREAMING"
echo "  1. Zero-Stat CAS Static Asset Server (--static) + BLAKE3 ETag 304 (< 2 us)"
echo "  2. Server-Sent Events (Slice.streamSSE / Response.sse) + Egress DLP"
echo "  3. App 11 (11-static-asset-cdn-edge) & App 12 (12-ai-sse-token-streamer)"
echo "========================================================================="

"$SLICE_BIN" pack "$APP11_DIR" "/tmp/f2_app11.cas" >/dev/null
"$SLICE_BIN" pack "$APP12_DIR" "/tmp/f2_app12.cas" >/dev/null
verify_app_parity "Phase F2 App 11: 11-static-asset-cdn-edge" "$APP11_DIR" "/tmp/f2_app11.cas" "-A"
verify_app_parity "Phase F2 App 12: 12-ai-sse-token-streamer" "$APP12_DIR" "/tmp/f2_app12.cas" "-A"

mkdir -p /tmp/f2_public/assets
cat << 'EOF' > /tmp/f2_public/index.html
<!doctype html><html><head><title>Slice F2 Static</title></head><body><h1>Zero-Stat CAS Web UI</h1></body></html>
EOF
cat << 'EOF' > /tmp/f2_public/assets/style.css
body { background: #090d16; color: #38bdf8; }
EOF

cat << 'EOF' > /tmp/f2_sse_server.ts
export default {
    fetch(req: Request): Response {
        const url = new URL(req.url);
        if (url.pathname === "/api/v1/stream") {
            return Response.sse([
                { id: "1", event: "token", data: { text: "Hello " } },
                { id: "2", event: "token", data: { text: "from Slice SSE" } },
                { id: "3", event: "done", data: "[DONE]" }
            ]);
        }
        return Response.json({ status: "ok" });
    }
};
EOF

"$SLICE_BIN" pack /tmp/f2_sse_server.ts /tmp/f2_fullstack.cas --static=/tmp/f2_public >/dev/null
"$SLICE_BIN" serve /tmp/f2_fullstack.cas --port=18092 --workers=4 --max-requests=6 >/dev/null 2>&1 &
F2_SRV_PID=$!
sleep 0.3

F2_INDEX_HDR=$(curl -fsSI "http://127.0.0.1:18092/")
F2_ETAG=$(echo "$F2_INDEX_HDR" | awk 'tolower($1)=="etag:" {gsub(/\r/,"",$2); print $2}')
F2_304_HDR=$(curl -fsSI -H "If-None-Match: $F2_ETAG" "http://127.0.0.1:18092/")
F2_SSE_BODY=$(curl -fsSL "http://127.0.0.1:18092/api/v1/stream")

kill "$F2_SRV_PID" 2>/dev/null || true
wait "$F2_SRV_PID" 2>/dev/null || true

echo "  [Live slice serve on Embedded CAS Static Assets + SSE Token Stream]"
echo "    Static / ETag    : $F2_ETAG"
echo "    304 Status Check : $(echo "$F2_304_HDR" | head -n 1 | tr -d '\r')"
if echo "$F2_304_HDR" | grep -q "304 Not Modified" && [[ "$F2_SSE_BODY" == *"from Slice SSE"* ]]; then
    echo "    [PASS] Zero-Stat CAS Static Asset ETag (HIT-200 + HIT-304) & SSE Streaming verified!"
else
    echo "    [FAIL] Unexpected static or SSE response!"
    exit 1
fi
rm -rf /tmp/f2_*
fi

if should_run_tag "f3"; then
echo "========================================================================="
echo "  [TAG: f3] PHASE F3 OCAP OUTBOUND FETCH(), SSRF SHIELD & OUTBOUND EGRESS DLP"
echo "  1. Real Outbound HTTP/1.1 fetch() with Embedded slice.toml allow_net"
echo "  2. Cloud Metadata (169.254.169.254) SSRF Shield & Outbound Egress DLP"
echo "  3. App 13 (13-webhook-orchestrator-ssrf) & App 14 (14-federated-api-mesh-dlp)"
echo "========================================================================="

"$SLICE_BIN" pack "$APP13_DIR" "/tmp/app13.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app13.cas" -o "/tmp/app13_bin" >/dev/null
"$SLICE_BIN" pack "$APP14_DIR" "/tmp/app14.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app14.cas" -o "/tmp/app14_bin" >/dev/null

verify_app_parity "Phase F3 App 13: 13-webhook-orchestrator-ssrf" "$APP13_DIR" "/tmp/app13.cas" "-A"
verify_app_parity "Phase F3 App 14: 14-federated-api-mesh-dlp" "$APP14_DIR" "/tmp/app14.cas" "-A"

bench_app 13 "13-WEBHOOK-ORCHESTRATOR-SSRF (Outbound fetch/SSRF/HMAC)" "$APP13_DIR" "-A"
bench_app 14 "14-FEDERATED-API-MESH-DLP (Outbound fetch/dlv/dset/DLP)" "$APP14_DIR" "-A"

echo "  --- Phase F3 Cold vs Warm Isolate Telemetry ---"
"$SLICE_BIN" run "/tmp/app13.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app13.cas" --warm --quiet --metrics
"$SLICE_BIN" run "/tmp/app14.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app14.cas" --warm --quiet --metrics

cat << 'EOF' > /tmp/f3_ssrf_dlp_check.ts
const ssrfAttempt = Slice.evalSandbox(
    "async function run() { await fetch('http://169.254.169.254/latest/meta-data/iam'); } run();",
    { allowNet: ["*"] }
);
const dlpAttempt = Slice.evalSandbox(
    "async function run() { const b64 = Buffer.from('sk_live_F3_GUARD_999').toString('base64'); await fetch('http://127.0.0.1:18095/upstream/webhook', { method: 'POST', headers: { 'X-Exfil': b64 }, body: '{}' }); } run();",
    { allowNet: ["127.0.0.1:18095"], auditSecret: "sk_live_F3_GUARD_999" }
);
console.log(JSON.stringify({
    ssrfBlocked: !ssrfAttempt.ok && String(ssrfAttempt.violation).indexOf("SSRF_METADATA_BLOCKED") !== -1,
    dlpBlocked: !dlpAttempt.ok && String(dlpAttempt.violation).indexOf("EGRESS_SECRET_LEAK_BLOCKED") !== -1
}));
EOF
F3_SEC_OUT=$("$SLICE_BIN" run /tmp/f3_ssrf_dlp_check.ts --allow-net=127.0.0.1:18095)
echo "  [Live SSRF Shield & Outbound Egress DLP Verification]"
echo "    Result : $F3_SEC_OUT"
if [[ "$F3_SEC_OUT" == '{"ssrfBlocked":true,"dlpBlocked":true}' ]]; then
    echo "    [PASS] Cloud Metadata SSRF Shield (169.254.169.254) & Outbound Egress Secret DLP verified!"
else
    echo "    [FAIL] Unexpected F3 security check output!"
    exit 1
fi
rm -f /tmp/f3_ssrf_dlp_check.ts
fi

if should_run_tag "f4"; then
echo "========================================================================="
echo "  [TAG: f4] PHASE F4 TIME-TRAVEL SLICE TEST, < 3MS --WATCH & AUTO-DLP .ENV"
echo "  1. Built-In Time-Travel Test Runner (slice test) with < 5us VFS+KV Rollbacks"
echo "  2. Auto-DLP .env Loader (--env-file) & < 3ms Live Dev Watcher (--watch-once)"
echo "  3. App 15 (15-time-travel-test-suite) & App 16 (16-dotenv-dlp-config-vault)"
echo "========================================================================="

"$SLICE_BIN" pack "$APP15_DIR" "/tmp/app15.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app15.cas" -o "/tmp/app15_bin" >/dev/null
"$SLICE_BIN" pack "$APP16_DIR" "/tmp/app16.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app16.cas" -o "/tmp/app16_bin" >/dev/null
"$SLICE_BIN" pack "$APP18_DIR" "/tmp/app18.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app18.cas" -o "/tmp/app18_bin" >/dev/null

verify_app_parity "Phase F4 App 15: 15-time-travel-test-suite" "$APP15_DIR" "/tmp/app15.cas" "-A"
verify_app_parity "Phase F4 App 16: 16-dotenv-dlp-config-vault" "$APP16_DIR" "/tmp/app16.cas" "-A"
verify_app_parity "Phase N2 App 18: 18-async-event-loop-fanout-scheduler" "$APP18_DIR" "/tmp/app18.cas" "-A"

bench_app 15 "15-TIME-TRAVEL-TEST-SUITE (Time-Travel VFS+KV/semver/dlv)" "$APP15_DIR" "-A"
bench_app 16 "16-DOTENV-DLP-CONFIG-VAULT (Auto-DLP .env/ini/cookie/bytes)" "$APP16_DIR" "-A"
bench_app 18 "18-ASYNC-EVENT-LOOP-FANOUT-SCHEDULER (Timer Wheel/Promise.all/AbortSignal)" "$APP18_DIR" "-A"

echo "  --- Phase F4 & Phase N-2 Cold vs Warm Isolate Telemetry ---"
"$SLICE_BIN" run "/tmp/app15.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app15.cas" --warm --quiet --metrics
"$SLICE_BIN" run "/tmp/app16.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app16.cas" --warm --quiet --metrics
"$SLICE_BIN" run "/tmp/app18.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app18.cas" --warm --quiet --metrics

echo "  [Live slice test Time-Travel Runner Verification on App 15]"
"$SLICE_BIN" test "$APP15_DIR"
F4_TEST_JSON=$("$SLICE_BIN" test "$APP15_DIR" --json)
if echo "$F4_TEST_JSON" | grep -q '"passed":3' && echo "$F4_TEST_JSON" | grep -q '"checkpointsRestored":3'; then
    echo "    [PASS] slice test Time-Travel Runner verified (3/3 passed, 3/3 VFS+KV rollbacks)!"
else
    echo "    [FAIL] Unexpected slice test JSON output: $F4_TEST_JSON"
    exit 1
fi
fi

if should_run_tag "app22"; then
echo "========================================================================="
echo "  [TAG: app22] APP 22 WORKER THREADS & RAW SOCKETS BENCHMARK"
echo "========================================================================="
"$SLICE_BIN" pack "$APP22_DIR" "/tmp/app22.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app22.cas" -o "/tmp/app22_bin" >/dev/null
verify_app_parity "App 22: 22-worker-threads-and-raw-sockets" "$APP22_DIR" "/tmp/app22.cas" "-A"
bench_app 22 "22-WORKER-THREADS-AND-RAW-SOCKETS (Worker Threads + TCP/UDP + 5-Ring Sec)" "$APP22_DIR" "-A"
measure_rss_kb "Slice (App 22: Workers + Sockets)"  "$SLICE_BIN" run "/tmp/app22.cas" --quiet
measure_rss_kb "Bun (App 22: Workers + Sockets)"    bun run "$APP22_DIR/src/index.ts"
measure_rss_kb "Deno (App 22: Workers + Sockets)"   deno run -A "$APP22_DIR/src/index.ts"
measure_rss_kb "Node (App 22: Workers + Sockets)"   node "$APP22_DIR/src/index.ts"
echo "  --- App 22 Cold vs Warm Isolate Telemetry ---"
"$SLICE_BIN" run "/tmp/app22.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app22.cas" --warm --quiet --metrics
fi

if should_run_tag "wasm"; then
echo "========================================================================="
echo "  [TAG: wasm] WAVE D WEBASSEMBLY (APPS 23 & 24) & OS-NATIVE WATCH MODE"
echo "========================================================================="
"$SLICE_BIN" pack "$APP23_DIR" "/tmp/app23.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app23.cas" -o "/tmp/app23_bin" >/dev/null
"$SLICE_BIN" pack "$APP24_DIR" "/tmp/app24.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app24.cas" -o "/tmp/app24_bin" >/dev/null
verify_app_parity "App 23: 23-wasm-crypto-and-simd-kernel" "$APP23_DIR" "/tmp/app23.cas" "-A"
verify_app_parity "App 24: 24-tsx-ssr-and-wasm-edge-pipeline" "$APP24_DIR" "/tmp/app24.cas" "-A"
bench_app 23 "23-WASM-CRYPTO-AND-SIMD-KERNEL (Pure-Rust WebAssembly + 5-Ring WASM Sec)" "$APP23_DIR" "-A"
bench_app 24 "24-TSX-SSR-AND-WASM-EDGE-PIPELINE (WebAssembly SLA Scorer + SSR Pipeline)" "$APP24_DIR" "-A"
measure_rss_kb "Slice (App 23: Pure-Rust WASM)"     "$SLICE_BIN" run "/tmp/app23.cas" --quiet
measure_rss_kb "Bun (App 23: Pure-Rust WASM)"       bun run "$APP23_DIR/src/index.ts"
measure_rss_kb "Deno (App 23: Pure-Rust WASM)"      deno run -A "$APP23_DIR/src/index.ts"
measure_rss_kb "Node (App 23: Pure-Rust WASM)"      node "$APP23_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 24: WASM + SSR)"         "$SLICE_BIN" run "/tmp/app24.cas" --quiet
measure_rss_kb "Bun (App 24: WASM + SSR)"           bun run "$APP24_DIR/src/index.ts"
measure_rss_kb "Deno (App 24: WASM + SSR)"          deno run -A "$APP24_DIR/src/index.ts"
measure_rss_kb "Node (App 24: WASM + SSR)"          node "$APP24_DIR/src/index.ts"
echo "  --- Apps 23 & 24 Cold vs Warm Isolate Telemetry ---"
"$SLICE_BIN" run "/tmp/app23.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app23.cas" --warm --quiet --metrics
"$SLICE_BIN" run "/tmp/app24.cas" --quiet --metrics
"$SLICE_BIN" run "/tmp/app24.cas" --warm --quiet --metrics
echo "  [Live slice test --watch-once (notify inotify/kqueue) on App 24]"
"$SLICE_BIN" test "$APP24_DIR" --watch-once
fi

if should_run_tag "frameworks"; then
echo "========================================================================="
echo "  [TAG: frameworks] WAVE E REAL-WORLD FRAMEWORK SUITES (HONO, ITTY+UNJS, EXPRESS)"
echo "========================================================================="
"$SLICE_BIN" pack "$APP25_DIR" "/tmp/app25.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app25.cas" -o "/tmp/app25_bin" >/dev/null
"$SLICE_BIN" pack "$APP26_DIR" "/tmp/app26.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app26.cas" -o "/tmp/app26_bin" >/dev/null
"$SLICE_BIN" pack "$APP32_DIR" "/tmp/app32.cas" >/dev/null
"$SLICE_BIN" compile "/tmp/app32.cas" -o "/tmp/app32_bin" >/dev/null
verify_app_parity "Framework 1 (App 25): 01-hono-edge-microservice" "$APP25_DIR" "/tmp/app25.cas" "-A"
verify_app_parity "Framework 2 (App 26): 02-itty-router-unjs-nitro-stack" "$APP26_DIR" "/tmp/app26.cas" "-A"
verify_app_parity "Framework 3 (App 32): 03-node-http-express-compat" "$APP32_DIR" "/tmp/app32.cas" "-A"
bench_app 25 "FRAMEWORK 1: 01-HONO-EDGE-MICROSERVICE (Hono v4.6.20 + hono/cookie + cookie + ms)" "$APP25_DIR" "-A"
bench_app 26 "FRAMEWORK 2: 02-ITTY-ROUTER-UNJS-NITRO-STACK (itty-router v5 + ufo + destr + ohash + radix3)" "$APP26_DIR" "-A"
bench_app 32 "FRAMEWORK 3: 03-NODE-HTTP-EXPRESS-COMPAT (node:http + node:stream Express/Polka)" "$APP32_DIR" "-A"
measure_rss_kb "Slice (FW 1: Hono v4 Microservice)" "$SLICE_BIN" run "/tmp/app25.cas" --quiet
measure_rss_kb "Bun (FW 1: Hono v4 Microservice)"   bun run "$APP25_DIR/src/index.ts"
measure_rss_kb "Deno (FW 1: Hono v4 Microservice)"  deno run -A "$APP25_DIR/src/index.ts"
measure_rss_kb "Node (FW 1: Hono v4 Microservice)"  node "$APP25_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (FW 2: itty-router + UnJS)"   "$SLICE_BIN" run "/tmp/app26.cas" --quiet
measure_rss_kb "Bun (FW 2: itty-router + UnJS)"     bun run "$APP26_DIR/src/index.ts"
measure_rss_kb "Deno (FW 2: itty-router + UnJS)"    deno run -A "$APP26_DIR/src/index.ts"
measure_rss_kb "Node (FW 2: itty-router + UnJS)"    node "$APP26_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (FW 3: Express node:http)"    "$SLICE_BIN" run "/tmp/app32.cas" --quiet
measure_rss_kb "Bun (FW 3: Express node:http)"      bun run "$APP32_DIR/src/index.ts"
measure_rss_kb "Deno (FW 3: Express node:http)"     deno run -A "$APP32_DIR/src/index.ts"
measure_rss_kb "Node (FW 3: Express node:http)"     node "$APP32_DIR/src/index.ts"
echo "  --- Frameworks 1..3 Cold vs Warm Isolate Telemetry ---"
"$SLICE_BIN" run "/tmp/app25.cas" --quiet --metrics
record_warm_metrics 25 "/tmp/app25.cas"
"$SLICE_BIN" run "/tmp/app26.cas" --quiet --metrics
record_warm_metrics 26 "/tmp/app26.cas"
"$SLICE_BIN" run "/tmp/app32.cas" --quiet --metrics
record_warm_metrics 32 "/tmp/app32.cas"
echo "  [Live slice test Runner Verification on Framework Suites]"
"$SLICE_BIN" test "$APP25_DIR"
"$SLICE_BIN" test "$APP26_DIR"
fi

if should_run_tag "case_studies"; then
echo "========================================================================="
echo "  [TAG: case_studies] FLAGSHIP PRODUCTION CASE STUDIES & WORKBENCH (CS 01-05)"
echo "========================================================================="
for cs_num in 27 28 29 30 31; do
    cs_var="APP${cs_num}_DIR"
    cs_dir="${!cs_var}"
    "$SLICE_BIN" pack "$cs_dir" "/tmp/app${cs_num}.cas" >/dev/null
    "$SLICE_BIN" compile "/tmp/app${cs_num}.cas" -o "/tmp/app${cs_num}_bin" >/dev/null
done

verify_app_parity "Case Study 1 (App 27): 01-tot-autonomous-sre-agent" "$APP27_DIR" "/tmp/app27.cas" "-A"
verify_app_parity "Case Study 2 (App 28): 02-mcp-code-mode-orchestrator" "$APP28_DIR" "/tmp/app28.cas" "-A"
verify_app_parity "Case Study 3 (App 29): 03-multi-tenant-saas-plugin-mesh" "$APP29_DIR" "/tmp/app29.cas" "-A"
verify_app_parity "Case Study 4 (App 30): 04-wasm-zero-trust-cdr-pipeline" "$APP30_DIR" "/tmp/app30.cas" "-A"
verify_app_parity "Case Study 5 (App 31): 05-time-travel-chaos-fuzzer-payments" "$APP31_DIR" "/tmp/app31.cas" "-A"

bench_app 27 "CS 1: 01-TOT-AUTONOMOUS-SRE-AGENT (Depth-4 ToT + 16 <5us Rollbacks + DLP/SES/Fuel)" "$APP27_DIR" "-A"
bench_app 28 "CS 2: 02-MCP-CODE-MODE-ORCHESTRATOR (40 MCP RPCs vs 1 Code Mode Pass, -98.4% Tokens)" "$APP28_DIR" "-A"
bench_app 29 "CS 3: 03-MULTI-TENANT-SAAS-PLUGIN-MESH (12 Tenants, 120 Events, Noisy-Neighbor + Hot-Swap)" "$APP29_DIR" "-A"
bench_app 30 "CS 4: 04-WASM-ZERO-TRUST-CDR-PIPELINE (Pure-Rust Wasm SimHash + PII Vault + Gzip)" "$APP30_DIR" "-A"
bench_app 31 "CS 5: 05-TIME-TRAVEL-CHAOS-FUZZER-PAYMENTS (48 Payment Sagas, 18 Rollbacks, 0-Cent Drift)" "$APP31_DIR" "-A"

for cs_num in 27 28 29 30 31; do
    cs_var="APP${cs_num}_DIR"
    cs_dir="${!cs_var}"
    cs_name="$(basename "$cs_dir")"
    echo "-------------------------------------------------------------------------"
    measure_rss_kb "Slice ($cs_name)" "$SLICE_BIN" run "/tmp/app${cs_num}.cas" --quiet
    measure_rss_kb "Bun   ($cs_name)" bun run "$cs_dir/src/index.ts"
    measure_rss_kb "Deno  ($cs_name)" deno run -A "$cs_dir/src/index.ts"
    measure_rss_kb "Node  ($cs_name)" node "$cs_dir/src/index.ts"
done

echo "  --- Case Studies 1..5 Cold vs Warm Isolate Telemetry ---"
for cs_num in 27 28 29 30 31; do
    "$SLICE_BIN" run "/tmp/app${cs_num}.cas" --quiet --metrics
    record_warm_metrics "$cs_num" "/tmp/app${cs_num}.cas"
done

echo "  [Live slice test Runner Verification on All 5 Case Studies]"
for cs_num in 27 28 29 30 31; do
    cs_var="APP${cs_num}_DIR"
    cs_dir="${!cs_var}"
    "$SLICE_BIN" test "$cs_dir"
done

echo "  [Live Interactive Workbench HTTP Server Smoke Verification]"
"$SLICE_BIN" serve "$WORKBENCH_DIR" --port=18091 --smoke-test
fi

if should_run_tag "swarm"; then
echo "========================================================================="
echo "  [TAG: swarm] MICROVM SPEEDUP RETENTION CHECK (1,000 TENANT SWARM)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000
fi

if [ -f "$ROOT_DIR/tests/scripts/generate_bench_report.mjs" ] && [ -f "$METRICS_JSONL" ]; then
    echo "========================================================================="
    echo "  AUTO-GENERATING TIMESTAMPED BENCHMARK REPORT & DOC SUMMARY"
    echo "========================================================================="
    node "$ROOT_DIR/tests/scripts/generate_bench_report.mjs" --jsonl "$METRICS_JSONL"
    if [ "${SLICE_SYNC_DOCS:-1}" = "1" ] && [ -f "$ROOT_DIR/tests/scripts/sync_doc_benchmarks.mjs" ]; then
        node "$ROOT_DIR/tests/scripts/sync_doc_benchmarks.mjs"
    fi
fi

rm -f /tmp/app*.cas /tmp/app*_bin "$METRICS_JSONL"
