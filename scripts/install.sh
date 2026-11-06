#!/usr/bin/env bash
# Slice (Nexus-JS) Linux / macOS Installer Script
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

INSTALL_DIR="${HOME}/.slice/bin"
mkdir -p "$INSTALL_DIR"

if [ ! -f "$ROOT_DIR/target/release/slice" ]; then
    echo "Compiling Slice v0.9.91 release binary..."
    cargo build --release
fi

cp -f "$ROOT_DIR/target/release/slice" "$INSTALL_DIR/slice"
chmod +x "$INSTALL_DIR/slice"

echo "Installed Slice v0.9.91 to $INSTALL_DIR/slice"
"$INSTALL_DIR/slice" info

