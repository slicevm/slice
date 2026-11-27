#!/usr/bin/env sh
# SliceVM Runtime (Linux & macOS) Installer Script
# Downloads prebuilt release archives with SHA-256 verification (default), or builds from source
# when `--from-source` or `--skip-build` is passed inside a local checkout.
set -eu

REPO="${SLICE_REPO:-slicevm/slice}"
INSTALL_DIR="${SLICE_INSTALL_DIR:-${HOME}/.slice/bin}"
VERSION="${SLICE_VERSION:-latest}"
MODE="prebuilt"

for arg in "$@"; do
    case "$arg" in
        --from-source) MODE="source" ;;
        --skip-build)  MODE="skip-build" ;;
        --version=*)   VERSION="${arg#--version=}" ;;
        *)
            echo "Unknown option: $arg" >&2
            echo "Usage: install.sh [--from-source] [--skip-build] [--version=vX.Y.Z]" >&2
            exit 1
            ;;
    esac
done

mkdir -p "$INSTALL_DIR"

if [ "$MODE" = "source" ] || [ "$MODE" = "skip-build" ]; then
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    cd "$ROOT_DIR"
    if [ "$MODE" = "source" ]; then
        echo "Building SliceVM Runtime release binary from source..."
        cargo build --release --locked
    elif [ ! -f "$ROOT_DIR/target/release/slice" ]; then
        echo "Error: --skip-build specified, but $ROOT_DIR/target/release/slice does not exist" >&2
        exit 1
    fi
    cp -f "$ROOT_DIR/target/release/slice" "$INSTALL_DIR/slice"
    chmod +x "$INSTALL_DIR/slice"
    echo "Installed SliceVM Runtime to $INSTALL_DIR/slice"
    "$INSTALL_DIR/slice" info
    exit 0
fi

OS_RAW="$(uname -s)"
ARCH_RAW="$(uname -m)"

case "$OS_RAW" in
    Linux)  OS="linux" ;;
    Darwin) OS="macos" ;;
    *)
        echo "Unsupported OS: $OS_RAW (use --from-source to build with Cargo)" >&2
        exit 1
        ;;
esac

case "$ARCH_RAW" in
    x86_64|amd64)  ARCH="x86_64" ;;
    arm64|aarch64) ARCH="aarch64" ;;
    *)
        echo "Unsupported architecture: $ARCH_RAW (use --from-source to build with Cargo)" >&2
        exit 1
        ;;
esac

ARCHIVE="slice-${OS}-${ARCH}.tar.gz"
if [ "$VERSION" = "latest" ]; then
    BASE_URL="https://github.com/${REPO}/releases/latest/download"
else
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

echo "Downloading ${ARCHIVE} from ${BASE_URL}..."
curl -fsSL "${BASE_URL}/${ARCHIVE}" -o "${TMP_DIR}/${ARCHIVE}"
curl -fsSL "${BASE_URL}/${ARCHIVE}.sha256" -o "${TMP_DIR}/${ARCHIVE}.sha256"

echo "Verifying SHA-256 checksum..."
EXPECTED_HASH="$(awk '{print $1}' "${TMP_DIR}/${ARCHIVE}.sha256" | tr 'A-F' 'a-f')"
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_HASH="$(sha256sum "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}' | tr 'A-F' 'a-f')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL_HASH="$(shasum -a 256 "${TMP_DIR}/${ARCHIVE}" | awk '{print $1}' | tr 'A-F' 'a-f')"
else
    echo "Error: neither sha256sum nor shasum found to verify archive checksum" >&2
    exit 1
fi

if [ "$EXPECTED_HASH" != "$ACTUAL_HASH" ]; then
    echo "Error: SHA-256 checksum mismatch for ${ARCHIVE}" >&2
    echo "  Expected: ${EXPECTED_HASH}" >&2
    echo "  Actual:   ${ACTUAL_HASH}" >&2
    exit 1
fi

tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "$TMP_DIR"
cp -f "${TMP_DIR}/slice" "${INSTALL_DIR}/slice"
chmod +x "${INSTALL_DIR}/slice"

echo "Installed SliceVM Runtime to ${INSTALL_DIR}/slice"
"${INSTALL_DIR}/slice" info
