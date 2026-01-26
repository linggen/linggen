#!/bin/bash
set -euo pipefail

# Build script for macOS (CLI and Server with embedded Web UI)
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

SLUG=$(detect_platform)
ARCH="$(uname -m)"

echo "🚀 Building Linggen ${VERSION} for macOS (${ARCH})"
echo "=========================================="

# Step 1: Build Frontend
echo "1️⃣  Building Frontend..."
cd "$ROOT_DIR/frontend"
if [ -f "package-lock.json" ]; then npm ci; else npm install; fi
npm run build
echo "✅ Frontend built"

# Step 2: Build CLI
echo ""
echo "2️⃣  Building CLI..."
cd "$ROOT_DIR/linggen-cli"
cargo clean -p linggen-cli
cargo build --release
BUILT_VER=$(target/release/linggen --version | awk '{print $2}')
if [ "$BUILT_VER" != "$VERSION_NUM" ]; then
  echo "❌ Error: Built CLI version ($BUILT_VER) does not match target version ($VERSION_NUM)" >&2
  exit 1
fi
tar -C target/release -czf "$DIST_DIR/linggen-cli-${SLUG}.tar.gz" linggen
echo "✅ CLI built: dist/linggen-cli-${SLUG}.tar.gz"

# Step 3: Build Server (with embedded UI)
echo ""
echo "3️⃣  Building Server..."
cd "$ROOT_DIR/backend"
cargo clean -p api
cargo build --release --bin linggen-server
SRV_VER=$(target/release/linggen-server --version | awk '{print $2}')
if [ "$SRV_VER" != "$VERSION_NUM" ]; then
  echo "❌ Error: Built server version ($SRV_VER) does not match target version ($VERSION_NUM)" >&2
  exit 1
fi

# Create server tarball (portable)
SRV_DIST_NAME="linggen-server-macos"
SRV_TEMP_DIR="$ROOT_DIR/dist-temp/$SRV_DIST_NAME"
rm -rf "$SRV_TEMP_DIR"
mkdir -p "$SRV_TEMP_DIR"
cp target/release/linggen-server "$SRV_TEMP_DIR/"
cd "$ROOT_DIR/dist-temp"
tar -czf "$DIST_DIR/${SRV_DIST_NAME}.tar.gz" "$SRV_DIST_NAME"
rm -rf "$SRV_TEMP_DIR"

echo "✅ Server built: dist/${SRV_DIST_NAME}.tar.gz"

# Step 4: Signing
echo ""
echo "4️⃣  Signing Artifacts..."

# Sign CLI
CLI_TARBALL="$DIST_DIR/linggen-cli-${SLUG}.tar.gz"
CLI_SIG=$(sign_file "$CLI_TARBALL" "$ROOT_DIR") || true
if [ -n "$CLI_SIG" ]; then
  echo "$CLI_SIG" > "${CLI_TARBALL}.sig.txt"
  echo "  ✅ CLI tarball signed"
fi

# Sign Server
SRV_TARBALL="$DIST_DIR/${SRV_DIST_NAME}.tar.gz"
SRV_SIG=$(sign_file "$SRV_TARBALL" "$ROOT_DIR") || true
if [ -n "$SRV_SIG" ]; then
  echo "$SRV_SIG" > "${SRV_TARBALL}.sig.txt"
  echo "  ✅ Server tarball signed"
fi

echo ""
echo "✅ macOS build complete! Artifacts are in $DIST_DIR"
