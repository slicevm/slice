#!/usr/bin/env bash
# Slice (Nexus-JS) Linux / macOS Installer Script
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

INSTALL_DIR="${HOME}/.slice/bin"
mkdir -p "$INSTALL_DIR"

if [[ "${1:-}" != "--skip-build" ]]; then
    echo "Compiling Slice v0.13.5 release binary..."
    cargo build --release --locked
elif [[ ! -f "$ROOT_DIR/target/release/slice" ]]; then
    echo "Error: --skip-build specified, but $ROOT_DIR/target/release/slice does not exist" >&2
    exit 1
fi

cp -f "$ROOT_DIR/target/release/slice" "$INSTALL_DIR/slice"
chmod +x "$INSTALL_DIR/slice"

echo "Installed Slice v0.13.5 to $INSTALL_DIR/slice"
"$INSTALL_DIR/slice" info

