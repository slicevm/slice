#!/usr/bin/env bash
# =============================================================================
# REAL-LIFE PRODUCTION CODE & REAL STACK BENCHMARK SUITE (Docker Linux x86_64)
# =============================================================================
# Compares Slice (Oxc TypeScript Compiler + Boa ECMAScript VM + Tier-0/2 Fast Paths)
# against:
#   - Bun (JavaScriptCore)
#   - Deno (V8)
#   - Node.js v22 (V8 + --experimental-strip-types)
#
# Workloads:
#   Stack 1 (`production_stack.ts`): E-Commerce & Fintech Checkout Microservice
#     - TypeScript interfaces, union types, classes, private properties,
#       functional `.map`/`.reduce`/`.some`, tax & discount calculation,
#       fraud risk heuristics, and JSON serialization across 50 checkout orders.
#   Stack 2 (`api_gateway_pipeline.ts`): Edge API Gateway, Auth & Rate Limiter
#     - HTTP request router, Bearer token verification, per-tenant sliding quota,
#       and structured telemetry digest across 100 API requests.
# =============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SLICE_BIN="$ROOT_DIR/target/release/slice"
PROD_TS="$ROOT_DIR/docker-tests/real-stack-app/src/production_stack.ts"
PROD_CAS="$ROOT_DIR/docker-tests/real-stack-app/production_stack.cas"
PROD_EXE="/tmp/slice_fintech_service"

GW_TS="$ROOT_DIR/docker-tests/real-stack-app/src/api_gateway_pipeline.ts"
GW_CAS="$ROOT_DIR/docker-tests/real-stack-app/api_gateway_pipeline.cas"
GW_EXE="/tmp/slice_gateway_service"

echo "========================================================================="
echo "  PACKING & COMPILING REAL-STACK PRODUCTION WORKLOADS (OXC + SLICE CAS)"
echo "========================================================================="
"$SLICE_BIN" pack "$PROD_TS" "$PROD_CAS"
"$SLICE_BIN" compile "$PROD_TS" -o "$PROD_EXE"
"$SLICE_BIN" pack "$GW_TS" "$GW_CAS"
"$SLICE_BIN" compile "$GW_TS" -o "$GW_EXE"

echo "========================================================================="
echo "  1. VERIFYING IDENTICAL PRODUCTION OUTPUT ACROSS ALL 4 RUNTIMES"
echo "========================================================================="
OUT_SLICE_1=$("$SLICE_BIN" run "$PROD_CAS")
OUT_BUN_1=$(bun run "$PROD_TS")
OUT_DENO_1=$(deno run "$PROD_TS")
OUT_NODE_1=$(node --experimental-strip-types "$PROD_TS" 2>/dev/null)

echo "  [Stack 1: Fintech Checkout Service]"
echo "  Slice Output : $OUT_SLICE_1"
echo "  Bun Output   : $OUT_BUN_1"
echo "  Deno Output  : $OUT_DENO_1"
echo "  Node Output  : $OUT_NODE_1"

if [ "$OUT_SLICE_1" = "$OUT_BUN_1" ] && [ "$OUT_SLICE_1" = "$OUT_DENO_1" ] && [ "$OUT_SLICE_1" = "$OUT_NODE_1" ]; then
    echo "  [PASS] Stack 1 Byte-for-Byte Identical Across Slice, Bun, Deno, and Node.js!"
else
    echo "  [FAIL] Stack 1 Output mismatch detected!"
    exit 1
fi

OUT_SLICE_2=$("$SLICE_BIN" run "$GW_CAS")
OUT_BUN_2=$(bun run "$GW_TS")
OUT_DENO_2=$(deno run "$GW_TS")
OUT_NODE_2=$(node --experimental-strip-types "$GW_TS" 2>/dev/null)

echo ""
echo "  [Stack 2: Edge API Gateway & Rate-Limiter Pipeline]"
echo "  Slice Output : $OUT_SLICE_2"
echo "  Bun Output   : $OUT_BUN_2"
echo "  Deno Output  : $OUT_DENO_2"
echo "  Node Output  : $OUT_NODE_2"

if [ "$OUT_SLICE_2" = "$OUT_BUN_2" ] && [ "$OUT_SLICE_2" = "$OUT_DENO_2" ] && [ "$OUT_SLICE_2" = "$OUT_NODE_2" ]; then
    echo "  [PASS] Stack 2 Byte-for-Byte Identical Across Slice, Bun, Deno, and Node.js!"
else
    echo "  [FAIL] Stack 2 Output mismatch detected!"
    exit 1
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
        printf "  %-32s | Min: %8.3f ms | Avg: %8.3f ms\n", name, min / 1000000.0, avg / 1000000.0
    }'
}

measure_rss_kb() {
    local name="$1"
    shift
    local rss
    rss=$(/usr/bin/time -v "$@" 2>&1 | grep "Maximum resident set size" | awk '{print $NF}')
    awk -v name="$name" -v rss="$rss" 'BEGIN {
        printf "  %-32s | Max RSS: %8d KB (%6.2f MB)\n", name, rss, rss / 1024.0
    }'
}

echo "========================================================================="
echo "  2. STACK 1 LATENCY: FINTECH CHECKOUT ENGINE (50 Orders, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"       20 "$PROD_EXE" --quiet
measure_ms "Slice (CAS + Oxc + Boa VM)"      20 "$SLICE_BIN" run "$PROD_CAS" --quiet
measure_ms "Slice (Raw .ts + Oxc + Boa VM)"  20 "$SLICE_BIN" run "$PROD_TS" --quiet
measure_ms "Bun v1.3.14"                     20 bun run "$PROD_TS"
measure_ms "Deno v2.8.3"                     20 deno run "$PROD_TS"
measure_ms "Node.js v22.22.3"                20 node --experimental-strip-types "$PROD_TS"

echo "========================================================================="
echo "  3. STACK 2 LATENCY: EDGE API GATEWAY PIPELINE (100 Requests, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"       20 "$GW_EXE" --quiet
measure_ms "Slice (CAS + Oxc + Boa VM)"      20 "$SLICE_BIN" run "$GW_CAS" --quiet
measure_ms "Slice (Raw .ts + Oxc + Boa VM)"  20 "$SLICE_BIN" run "$GW_TS" --quiet
measure_ms "Bun v1.3.14"                     20 bun run "$GW_TS"
measure_ms "Deno v2.8.3"                     20 deno run "$GW_TS"
measure_ms "Node.js v22.22.3"                20 node --experimental-strip-types "$GW_TS"

echo "========================================================================="
echo "  4. REAL PRODUCTION STACK PEAK MEMORY FOOTPRINT (RSS)"
echo "========================================================================="
measure_rss_kb "Slice (Fintech CAS + Boa VM)" "$SLICE_BIN" run "$PROD_CAS" --quiet
measure_rss_kb "Slice (Gateway CAS + Boa VM)" "$SLICE_BIN" run "$GW_CAS" --quiet
measure_rss_kb "Bun v1.3.14 (Fintech .ts)"    bun run "$PROD_TS"
measure_rss_kb "Deno v2.8.3 (Fintech .ts)"    deno run "$PROD_TS"
measure_rss_kb "Node.js v22.22.3 (Fintech)"   node --experimental-strip-types "$PROD_TS"

echo "========================================================================="
echo "  5. SLICE INTERNAL ISOLATE TELEMETRY ON PRODUCTION STACKS"
echo "========================================================================="
"$SLICE_BIN" run "$PROD_CAS" --metrics
"$SLICE_BIN" run "$GW_CAS" --metrics

echo "========================================================================="
echo "  6. MICROVM SPEEDUP RETENTION CHECK (1,000 TENANT SWARM)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000

rm -f "$PROD_EXE" "$GW_EXE"
