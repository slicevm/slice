#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TAGS="${1:-${SLICE_BENCH_TAGS:-all}}"
VERSION="$(awk -F'"' '/^\[workspace\.package\]/{f=1} f&&/^version/{print $2; exit}' Cargo.toml)"
COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")"
TIMESTAMP_UTC="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
REPORTS_DIR="$ROOT_DIR/tests/reports"
mkdir -p "$REPORTS_DIR"

echo "========================================================================="
echo "  SLICE AUTOMATED DOCKER BENCHMARK & REPORT RUNNER"
echo "  Version : v$VERSION | Commit : $COMMIT | Tags : $TAGS"
echo "========================================================================="

if [ "${SKIP_DOCKER_BUILD:-0}" != "1" ]; then
    echo "[1/3] Building Linux x86_64 Docker benchmark image..."
    docker build -f tests/Dockerfile -t slice-real-stack-bench .
fi

echo "[2/3] Running benchmarks and generating timestamped report in tests/reports/..."
docker run --rm \
    -e "SLICE_BENCH_TAGS=$TAGS" \
    -e "SLICE_VERSION=$VERSION" \
    -e "SLICE_GIT_COMMIT=$COMMIT" \
    -e "SLICE_BENCH_TS_UTC=$TIMESTAMP_UTC" \
    -e "SLICE_SYNC_DOCS=0" \
    -v "$REPORTS_DIR:/app/tests/reports" \
    slice-real-stack-bench

echo "[3/3] Synchronizing summarized benchmark tables across README.md and docs/..."
node tests/scripts/sync_doc_benchmarks.mjs

echo "Done! Latest report: tests/reports/latest.md"
