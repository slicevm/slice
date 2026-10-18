#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SLICE_BIN="$ROOT_DIR/target/release/slice"
COLD_TS="$ROOT_DIR/benchmarks/workloads/cold_start.ts"
COLD_CAS="$ROOT_DIR/benchmarks/workloads/cold_start.cas"
COMPUTE_JS="$ROOT_DIR/benchmarks/workloads/compute_heavy.js"
COMPUTE_CAS="$ROOT_DIR/benchmarks/workloads/compute_heavy.cas"
MOD_DIR="$ROOT_DIR/benchmarks/workloads/module_graph/generated"
MOD_CAS="$ROOT_DIR/benchmarks/workloads/module_graph/bundle500.cas"

mkdir -p "$MOD_DIR"
INDEX_FILE="$MOD_DIR/index.mjs"
> "$INDEX_FILE"
for i in $(seq 0 499); do
    echo "export const VAL_${i} = $((i + 1));" > "$MOD_DIR/mod_${i}.mjs"
    echo "import { VAL_${i} } from './mod_${i}.mjs';" >> "$INDEX_FILE"
done
echo "console.log(VAL_499);" >> "$INDEX_FILE"

"$SLICE_BIN" pack "$COLD_TS" "$COLD_CAS" > /dev/null
"$SLICE_BIN" pack "$COMPUTE_JS" "$COMPUTE_CAS" > /dev/null
"$SLICE_BIN" pack "$MOD_DIR" "$MOD_CAS" --entry=./index.mjs > /dev/null

measure_ms() {
    local name="$1"
    local runs="$2"
    shift 2
    # Warmup
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
        printf "  %-26s | Min: %8.3f ms | Avg: %8.3f ms\n", name, min / 1000000.0, avg / 1000000.0
    }'
}

measure_rss_kb() {
    local name="$1"
    shift
    local rss
    rss=$(/usr/bin/time -v "$@" 2>&1 | grep "Maximum resident set size" | awk '{print $NF}')
    awk -v name="$name" -v rss="$rss" 'BEGIN {
        printf "  %-26s | Max RSS: %8d KB (%6.2f MB)\n", name, rss, rss / 1024.0
    }'
}

echo "========================================================================="
echo "  LINUX KERNEL ENVIRONMENT"
echo "========================================================================="
uname -a
"$SLICE_BIN" info

echo "========================================================================="
echo "  1. CLI PROCESS COLD-START BENCHMARK (TypeScript Execution, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (CAS Snapshot)" 20 "$SLICE_BIN" run "$COLD_CAS" --quiet
measure_ms "Slice (Raw .ts)"      20 "$SLICE_BIN" run "$COLD_TS" --quiet
measure_ms "Bun"                  20 bun run "$COLD_TS"
measure_ms "Deno"                 20 deno run "$COLD_TS"
measure_ms "Node.js"              20 node --experimental-strip-types "$COLD_TS"

echo "========================================================================="
echo "  2. 500-MODULE GRAPH RESOLUTION (Zero-Stat CAS vs POSIX stat/read)"
echo "========================================================================="
measure_ms "Slice (Zero-Stat CAS)" 10 "$SLICE_BIN" run "$MOD_CAS" --quiet
measure_ms "Bun"                   10 bun run "$INDEX_FILE"
measure_ms "Deno"                  10 deno run "$INDEX_FILE"
measure_ms "Node.js"               10 node "$INDEX_FILE"

echo "========================================================================="
echo "  3. COMPUTE KERNEL BENCHMARK (Fib(40) + 10,000,000 Loop)"
echo "========================================================================="
measure_ms "Slice (Tier-2 Native)" 15 "$SLICE_BIN" run "$COMPUTE_CAS" --quiet
measure_ms "Bun"                   15 bun run "$COMPUTE_JS"
measure_ms "Deno"                  15 deno run "$COMPUTE_JS"
measure_ms "Node.js"               15 node "$COMPUTE_JS"

echo "========================================================================="
echo "  4. BASELINE MEMORY FOOTPRINT (PEAK RSS VIA /usr/bin/time -v)"
echo "========================================================================="
measure_rss_kb "Slice (CAS Snapshot)" "$SLICE_BIN" run "$COLD_CAS" --quiet
measure_rss_kb "Bun"                  bun run "$COLD_TS"
measure_rss_kb "Deno"                 deno run "$COLD_TS"
measure_rss_kb "Node.js"              node --experimental-strip-types "$COLD_TS"

echo "========================================================================="
echo "  5. SLICE IN-PROCESS MICROVM SWARM (1,000 Isolated Tenants)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000
"$SLICE_BIN" verify-security

echo "========================================================================="
echo "  6. PRODUCTION TOOLING VERIFICATION (doctor, inspect, serve, compile)"
echo "========================================================================="
"$SLICE_BIN" doctor
"$SLICE_BIN" inspect "$COLD_CAS"
"$SLICE_BIN" serve "$COLD_CAS" --smoke-test
"$SLICE_BIN" compile "$COLD_TS" -o /tmp/slice_standalone_demo
/tmp/slice_standalone_demo --metrics
rm -f /tmp/slice_standalone_demo


