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
# 8 Folder-Based Production Test Applications (`docker-tests/apps/`):
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
APP5_DIR="$APPS_DIR/05-jwt-oauth-session-guard"
APP6_DIR="$APPS_DIR/06-telemetry-metrics-pipeline"
APP7_DIR="$APPS_DIR/07-graphql-ast-validator"
APP8_DIR="$APPS_DIR/08-event-driven-state-journal"

echo "========================================================================="
echo "  RUNTIME VERSIONS (LINUX DOCKER x86_64 - LATEST RELEASES)"
echo "========================================================================="
echo "  Slice   : v0.2.0 (Oxc TS Compiler + Boa ES2024 VM + Zero-Stat CAS)"
echo "  Node.js : $(node -v)"
echo "  Bun     : v$(bun -v)"
echo "  Deno    : $(deno -V)"

echo "========================================================================="
echo "  PACKING & COMPILING ALL 8 FOLDER-BASED APPS (OXC + NODE_MODULES + CAS)"
echo "========================================================================="
for i in 1 2 3 4 5 6 7 8; do
    eval "dir=\$APP${i}_DIR"
    "$SLICE_BIN" pack "$dir" "/tmp/app${i}.cas"
    "$SLICE_BIN" compile "/tmp/app${i}.cas" -o "/tmp/app${i}_bin"
done

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
echo "  1. VERIFYING 100% OUTPUT PARITY ACROSS ALL 8 FOLDER APPS & 4 RUNTIMES"
echo "========================================================================="
verify_app_parity "App 1: 01-fintech-checkout (Multi-File TS + SHA-256)" "$APP1_DIR" "/tmp/app1.cas" ""
verify_app_parity "App 2: 02-npm-ecosystem-suite (semver + ms + ini)" "$APP2_DIR" "/tmp/app2.cas" "-A"
verify_app_parity "App 3: 03-disk-io-log-pipeline (10 JSONL Shards Disk I/O)" "$APP3_DIR" "/tmp/app3.cas" "-A"
verify_app_parity "App 4: 04-edge-api-gateway (fast-querystring + ms + Auth)" "$APP4_DIR" "/tmp/app4.cas" "-A"
verify_app_parity "App 5: 05-jwt-oauth-session-guard (cookie + ms + HMAC-SHA256)" "$APP5_DIR" "/tmp/app5.cas" "-A"
verify_app_parity "App 6: 06-telemetry-metrics-pipeline (bytes + flatted + WAL)" "$APP6_DIR" "/tmp/app6.cas" "-A"
verify_app_parity "App 7: 07-graphql-ast-validator (dlv + dset + AST Cost Guard)" "$APP7_DIR" "/tmp/app7.cas" "-A"
verify_app_parity "App 8: 08-event-driven-state-journal (mitt + ms + JSONL Replay)" "$APP8_DIR" "/tmp/app8.cas" "-A"

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

bench_app 1 "01-FINTECH-CHECKOUT (Multi-File TS + SHA-256)" "$APP1_DIR" ""
bench_app 2 "02-NPM-ECOSYSTEM-SUITE (semver + ms + ini)" "$APP2_DIR" "-A"
bench_app 3 "03-DISK-IO-LOG-PIPELINE (10 JSONL Shards I/O)" "$APP3_DIR" "-A"
bench_app 4 "04-EDGE-API-GATEWAY (fast-querystring + ms)" "$APP4_DIR" "-A"
bench_app 5 "05-JWT-OAUTH-SESSION-GUARD (cookie + ms + HMAC)" "$APP5_DIR" "-A"
bench_app 6 "06-TELEMETRY-METRICS-PIPELINE (bytes + flatted)" "$APP6_DIR" "-A"
bench_app 7 "07-GRAPHQL-AST-VALIDATOR (dlv + dset + AST)" "$APP7_DIR" "-A"
bench_app 8 "08-EVENT-DRIVEN-STATE-JOURNAL (mitt + ms + WAL)" "$APP8_DIR" "-A"

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
echo "  SLICE INTERNAL ISOLATE TELEMETRY ACROSS ALL 8 APPS (COLD VS WARM POOL)"
echo "========================================================================="
echo "  --- Cold Realm Telemetry ---"
for i in 1 2 3 4 5 6 7 8; do
    "$SLICE_BIN" run "/tmp/app${i}.cas" --quiet --metrics
done
echo "  --- Pre-Warmed Boa Pool Telemetry (--warm) ---"
for i in 1 2 3 4 5 6 7 8; do
    "$SLICE_BIN" run "/tmp/app${i}.cas" --warm --quiet --metrics
done

echo "========================================================================="
echo "  MICROVM SPEEDUP RETENTION CHECK (1,000 TENANT SWARM)"
echo "========================================================================="
"$SLICE_BIN" microvm-bench --tenants=1000

rm -f /tmp/app*.cas /tmp/app*_bin
