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
# Folder-Based Production Test Applications (`docker-tests/apps/`):
#   1. `01-fintech-checkout`:
#      Multi-module TypeScript OOP/Functional checkout engine + `node:crypto` SHA-256
#   2. `02-npm-ecosystem-suite`:
#      Real external npm libraries from `node_modules/` (`semver@7.6.3`, `ms@2.1.3`, `ini@4.1.3`)
#   3. `03-disk-io-log-pipeline`:
#      Intensive disk & filesystem I/O (`node:fs`, `node:path`, `node:os`, `node:crypto`)
#      writing, stating, scanning, reading, parsing, and verifying 20 JSONL log shards (500 records)
#   4. `04-edge-api-gateway`:
#      Multi-module Edge API Gateway + external npm `fast-querystring@1.1.2` & `ms@2.1.3`
# =============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

SLICE_BIN="$ROOT_DIR/target/release/slice"
APPS_DIR="$ROOT_DIR/docker-tests/apps"

APP1_DIR="$APPS_DIR/01-fintech-checkout"
APP2_DIR="$APPS_DIR/02-npm-ecosystem-suite"
APP3_DIR="$APPS_DIR/03-disk-io-log-pipeline"
APP4_DIR="$APPS_DIR/04-edge-api-gateway"

echo "========================================================================="
echo "  RUNTIME VERSIONS (LINUX DOCKER x86_64 - LATEST RELEASES)"
echo "========================================================================="
echo "  Slice   : v0.2.0 (Oxc TS Compiler + Boa ES2024 VM + Zero-Stat CAS)"
echo "  Node.js : $(node -v)"
echo "  Bun     : v$(bun -v)"
echo "  Deno    : $(deno -V)"

echo "========================================================================="
echo "  PACKING & COMPILING 4 FOLDER-BASED APPS (OXC + NODE_MODULES + CAS)"
echo "========================================================================="
"$SLICE_BIN" pack "$APP1_DIR" "/tmp/app1.cas"
"$SLICE_BIN" compile "/tmp/app1.cas" -o "/tmp/app1_bin"

"$SLICE_BIN" pack "$APP2_DIR" "/tmp/app2.cas"
"$SLICE_BIN" compile "/tmp/app2.cas" -o "/tmp/app2_bin"

"$SLICE_BIN" pack "$APP3_DIR" "/tmp/app3.cas"
"$SLICE_BIN" compile "/tmp/app3.cas" -o "/tmp/app3_bin"

"$SLICE_BIN" pack "$APP4_DIR" "/tmp/app4.cas"
"$SLICE_BIN" compile "/tmp/app4.cas" -o "/tmp/app4_bin"

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

echo "========================================================================="
echo "  1. VERIFYING 100% OUTPUT PARITY ACROSS ALL 4 FOLDER APPS & 4 RUNTIMES"
echo "========================================================================="
verify_app_parity "App 1: 01-fintech-checkout (Multi-File TS + SHA-256)" "$APP1_DIR" "/tmp/app1.cas" ""
verify_app_parity "App 2: 02-npm-ecosystem-suite (semver + ms + ini)" "$APP2_DIR" "/tmp/app2.cas" "-A"
verify_app_parity "App 3: 03-disk-io-log-pipeline (10 JSONL Shards Disk I/O)" "$APP3_DIR" "/tmp/app3.cas" "-A"
verify_app_parity "App 4: 04-edge-api-gateway (fast-querystring + ms + Auth)" "$APP4_DIR" "/tmp/app4.cas" "-A"

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

echo "========================================================================="
echo "  2. APP 1 LATENCY: 01-FINTECH-CHECKOUT (Multi-File TS + SHA-256, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"           20 "/tmp/app1_bin" --quiet
measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app1.cas" --quiet
measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$APP1_DIR" --quiet
measure_ms "Bun (Latest)"                        20 bun run "$APP1_DIR/src/index.ts"
measure_ms "Deno (Latest)"                       20 deno run "$APP1_DIR/src/index.ts"
measure_ms "Node.js (Latest)"                    20 node "$APP1_DIR/src/index.ts"

echo "========================================================================="
echo "  3. APP 2 LATENCY: 02-NPM-ECOSYSTEM-SUITE (semver + ms + ini, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"           20 "/tmp/app2_bin" --quiet
measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app2.cas" --quiet
measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$APP2_DIR" --quiet
measure_ms "Bun (Latest)"                        20 bun run "$APP2_DIR/src/index.ts"
measure_ms "Deno (Latest)"                       20 deno run -A "$APP2_DIR/src/index.ts"
measure_ms "Node.js (Latest)"                    20 node "$APP2_DIR/src/index.ts"

echo "========================================================================="
echo "  4. APP 3 LATENCY: 03-DISK-IO-LOG-PIPELINE (10 JSONL Shards I/O, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"           20 "/tmp/app3_bin" --quiet
measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app3.cas" --quiet
measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$APP3_DIR" --quiet
measure_ms "Bun (Latest)"                        20 bun run "$APP3_DIR/src/index.ts"
measure_ms "Deno (Latest)"                       20 deno run -A "$APP3_DIR/src/index.ts"
measure_ms "Node.js (Latest)"                    20 node "$APP3_DIR/src/index.ts"

echo "========================================================================="
echo "  5. APP 4 LATENCY: 04-EDGE-API-GATEWAY (fast-querystring + ms, 20 Runs)"
echo "========================================================================="
measure_ms "Slice (Standalone Binary)"           20 "/tmp/app4_bin" --quiet
measure_ms "Slice (CAS Archive)"                 20 "$SLICE_BIN" run "/tmp/app4.cas" --quiet
measure_ms "Slice (Folder Source + Oxc)"         20 "$SLICE_BIN" run "$APP4_DIR" --quiet
measure_ms "Bun (Latest)"                        20 bun run "$APP4_DIR/src/index.ts"
measure_ms "Deno (Latest)"                       20 deno run -A "$APP4_DIR/src/index.ts"
measure_ms "Node.js (Latest)"                    20 node "$APP4_DIR/src/index.ts"

echo "========================================================================="
echo "  6. PEAK MEMORY FOOTPRINT (MAX RSS) ACROSS ALL 4 FOLDER APPS"
echo "========================================================================="
measure_rss_kb "Slice (App 2: NPM semver+ms+ini)"  "$SLICE_BIN" run "/tmp/app2.cas" --quiet
measure_rss_kb "Bun (App 2: NPM semver+ms+ini)"    bun run "$APP2_DIR/src/index.ts"
measure_rss_kb "Deno (App 2: NPM semver+ms+ini)"   deno run -A "$APP2_DIR/src/index.ts"
measure_rss_kb "Node (App 2: NPM semver+ms+ini)"   node "$APP2_DIR/src/index.ts"
echo "-------------------------------------------------------------------------"
measure_rss_kb "Slice (App 3: Disk I/O 10 Shards)" "$SLICE_BIN" run "/tmp/app3.cas" --quiet
measure_rss_kb "Bun (App 3: Disk I/O 10 Shards)"   bun run "$APP3_DIR/src/index.ts"
measure_rss_kb "Deno (App 3: Disk I/O 10 Shards)"  deno run -A "$APP3_DIR/src/index.ts"
measure_rss_kb "Node (App 3: Disk I/O 10 Shards)"  node "$APP3_DIR/src/index.ts"

echo "========================================================================="
echo "  7. SLICE INTERNAL ISOLATE TELEMETRY ON ALL 4 FOLDER APPS"
echo "========================================================================="
"$SLICE_BIN" run "/tmp/app1.cas" --metrics
"$SLICE_BIN" run "/tmp/app2.cas" --metrics
"$SLICE_BIN" run "/tmp/app3.cas" --metrics
"$SLICE_BIN" run "/tmp/app4.cas" --metrics

echo "========================================================================="
echo "  8. MICROVM SPEEDUP RETENTION CHECK (1,000 TENANT SWARM)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000

rm -f /tmp/app1.cas /tmp/app2.cas /tmp/app3.cas /tmp/app4.cas /tmp/app1_bin /tmp/app2_bin /tmp/app3_bin /tmp/app4_bin
