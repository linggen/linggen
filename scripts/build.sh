#!/bin/bash
set -euo pipefail

# Master build orchestrator script for Linggen
# Usage: ./scripts/build.sh <version> [--skip-linux]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

VERSION=""
SKIP_LINUX=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-linux)
      SKIP_LINUX=true
      shift ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      fi
      shift ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--skip-linux]" >&2
  exit 1
fi

VERSION_NUM="${VERSION#v}"

echo "🏗️  Building Linggen ${VERSION}"
echo "=============================="

# 0. Sync version to all project files
echo "🔄 Syncing version $VERSION_NUM to all project files..."
"$ROOT_DIR/scripts/sync-version.sh" "$VERSION_NUM"

# 0.5 Clean dist/ to avoid uploading stale artifacts if a later step doesn't produce a new file.
# (release.sh uploads whatever exists in dist/, so a missing build output could accidentally ship an old binary)
echo "🧹 Cleaning dist/..."
rm -rf "$ROOT_DIR/dist"
mkdir -p "$ROOT_DIR/dist"

# 1. Build local platform artifacts
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
if [ "$OS" = "darwin" ]; then
  echo "📦 Step 1: Building macOS artifacts..."
  "$ROOT_DIR/scripts/build-mac.sh" "$VERSION"
else
  echo "📦 Step 1: Building local Linux artifacts (CLI & Server)..."
  # On Linux, the multi-arch docker build covers most needs, 
  # but we can do a local build here if needed.
  cd "$ROOT_DIR/linggen-cli" && cargo clean -p linggen-cli && cargo build --release
  cd "$ROOT_DIR/backend" && cargo clean -p api && cargo build --release --bin linggen-server
fi

# 2. Build multi-arch Linux artifacts (requires Docker)
if [ "$SKIP_LINUX" = "true" ]; then
  echo ""
  echo "⏩ Step 2: Skipping multi-arch Linux build."
else
  if command -v docker >/dev/null && docker buildx version >/dev/null 2>&1; then
    echo ""
    echo "🐳 Step 2: Building multi-arch Linux packages via Docker..."
    "$ROOT_DIR/scripts/build-linux.sh" "$VERSION"
  else
    echo ""
    echo "⚠️  Docker or Buildx not found. Skipping multi-arch Linux build."
  fi
fi

echo ""
echo "✅ Build complete! All artifacts are in the dist/ directory."
