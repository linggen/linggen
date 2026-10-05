#!/bin/bash
set -euo pipefail

# Build script for macOS — builds the 'ling' CLI binary with embedded Web UI
# Usage: ./scripts/build-mac.sh <version>

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>" >&2
  exit 1
fi

VERSION_NUM="${VERSION#v}"
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"

ensure_cargo

SLUG=$(detect_platform)
ARCH="$(uname -m)"

echo "🚀 Building Linggen ${VERSION} for macOS (${ARCH})"
echo "=========================================="

# Step 1: Build Web UI
echo "1️⃣  Building Web UI..."
cd "$ROOT_DIR/ui"
if [ -f "package-lock.json" ]; then npm ci; else npm install; fi
npm run build
echo "✅ Web UI built"

# Step 2: Build ling binary (with embedded UI via rust-embed)
echo ""
echo "2️⃣  Building ling binary..."
cd "$ROOT_DIR"
cargo clean -p linggen
cargo build --release
BUILT_VER=$(target/release/ling --version | awk '{print $2}')
if [ "$BUILT_VER" != "$VERSION_NUM" ]; then
  echo "❌ Error: Built version ($BUILT_VER) does not match target version ($VERSION_NUM)" >&2
  exit 1
fi
# Sign explicitly — macOS 26.x SIGKILLs ("Code Signature Invalid") the
# linker-signed binary cargo emits when it runs from a tarball. Developer ID +
# hardened runtime + notarization when the identity is in the keychain, so a
# browser-downloaded (quarantined) `ling` passes Gatekeeper; ad-hoc otherwise.
echo "🔏 Codesigning..."
"$ROOT_DIR/scripts/sign-mac.sh" target/release/ling "$ROOT_DIR/scripts/ling.entitlements"

tar -C target/release -czf "$DIST_DIR/ling-${SLUG}.tar.gz" ling
echo "✅ ling built: dist/ling-${SLUG}.tar.gz"

echo ""
echo "✅ macOS build complete! Artifacts are in $DIST_DIR"
