#!/usr/bin/env bash
# Slice (Nexus-JS) Single-Source Version Bump Script (Bash)
# Usage:
#   ./scripts/bump_version.sh 0.9.2     # Set explicit version
#   ./scripts/bump_version.sh patch     # Increment patch (default if omitted)
#   ./scripts/bump_version.sh minor     # Increment minor (resets patch to 0)
#   ./scripts/bump_version.sh major     # Increment major (resets minor & patch to 0)

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGET="${1:-patch}"
OLD_VERSION="$(grep -A1 '^\[workspace\.package\]' Cargo.toml | grep '^version' | sed -E 's/version = "([0-9]+\.[0-9]+\.[0-9]+)"/\1/')"

if [[ -z "$OLD_VERSION" ]]; then
    echo "Error: Could not parse [workspace.package] version from Cargo.toml" >&2
    exit 1
fi

IFS='.' read -r CUR_MAJOR CUR_MINOR CUR_PATCH <<< "$OLD_VERSION"

case "$TARGET" in
    patch)
        NEW_VERSION="${CUR_MAJOR}.${CUR_MINOR}.$((CUR_PATCH + 1))"
        ;;
    minor)
        NEW_VERSION="${CUR_MAJOR}.$((CUR_MINOR + 1)).0"
        ;;
    major)
        NEW_VERSION="$((CUR_MAJOR + 1)).0.0"
        ;;
    *)
        if [[ ! "$TARGET" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
            echo "Error: Invalid version target '$TARGET'. Expected 'patch', 'minor', 'major', or 'X.Y.Z'." >&2
            exit 1
        fi
        NEW_VERSION="$TARGET"
        ;;
esac

if [[ "$OLD_VERSION" == "$NEW_VERSION" ]]; then
    echo "Version is already $NEW_VERSION (nothing to change)."
    exit 0
fi

echo "Bumping Slice codebase version: v${OLD_VERSION} -> v${NEW_VERSION}"

# 1. Update Cargo.toml
sed -i.bak -E "/^\[workspace\.package\]/,/^\[/ s/^version = \"${OLD_VERSION}\"/version = \"${NEW_VERSION}\"/" Cargo.toml
rm -f Cargo.toml.bak
echo "  [updated] Cargo.toml"

# 2. Update scripts and docs (never touching tests/apps/* or tests/frameworks/* fixtures)
FILES=(
    "scripts/install.ps1"
    "scripts/install.sh"
    "scripts/package_release.ps1"
    "scripts/run_benchmarks.ps1"
    "tests/run_real_stack_benchmarks.sh"
    "tests/scripts/generate_bench_report.mjs"
    "README.md"
    "tests/README.md"
    "tests/frameworks/README.md"
    docs/general/*.md
    docs/guides/*.md
    docs/system/*.md
)

for f in "${FILES[@]}"; do
    if [[ -f "$f" ]]; then
        if grep -qE "v${OLD_VERSION}|\"version\": ?\"${OLD_VERSION}\"|SLICE_VERSION:-${OLD_VERSION}|\$Version = \"${OLD_VERSION}\"|\|\| \"${OLD_VERSION}\"|Slice-NexusJS/${OLD_VERSION}" "$f"; then
            sed -i.bak -E \
                -e "/<!-- BENCHMARK_SUMMARY_START -->/,/<!-- BENCHMARK_SUMMARY_END -->/! s/\bv${OLD_VERSION}\b/v${NEW_VERSION}/g" \
                -e "s/\"version\": \"${OLD_VERSION}\"/\"version\": \"${NEW_VERSION}\"/g" \
                -e "s/\"version\":\"${OLD_VERSION}\"/\"version\":\"${NEW_VERSION}\"/g" \
                -e "s/SLICE_VERSION:-${OLD_VERSION}\b/SLICE_VERSION:-${NEW_VERSION}/g" \
                -e "s/\\\$Version = \"${OLD_VERSION}\"/\$Version = \"${NEW_VERSION}\"/g" \
                -e "s/\|\| \"${OLD_VERSION}\"/|| \"${NEW_VERSION}\"/g" \
                -e "s/Slice-NexusJS\/${OLD_VERSION}\b/Slice-NexusJS\/${NEW_VERSION}/g" \
                "$f"
            rm -f "${f}.bak"
            echo "  [updated] $f"
        fi
    fi
done

echo "  [syncing] Cargo.lock via cargo update --workspace..."
cargo update --workspace --quiet
echo "Successfully bumped Slice to v${NEW_VERSION}"

