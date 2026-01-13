#!/bin/bash
set -euo pipefail

# Master build orchestrator script for Linggen
# Usage: ./scripts/build.sh <version> [--skip-linux]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

VERSION="${1:-}"
SKIP_LINUX=false

# Check if arguments were provided at all
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--skip-linux]" >&2
  exit 1
fi

if [ "$VERSION" = "--skip-linux" ]; then
  SKIP_LINUX=true
  VERSION="${2:-}"
  if [ -z "$VERSION" ]; then
    echo "Error: Version required when using --skip-linux" >&2
    echo "Usage: $0 <version> [--skip-linux]" >&2
    exit 1
  fi
fi

echo "🏗️  Building Linggen ${VERSION}"
echo "=============================="

# 1. Build local platform artifacts
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
if [ "$OS" = "darwin" ]; then
  echo "📦 Step 1: Building macOS artifacts..."
  "$ROOT_DIR/scripts/build-mac.sh" "$VERSION"
else
  echo "📦 Step 1: Building local Linux artifacts (CLI & Server)..."
  # On Linux, the multi-arch docker build covers most needs, 
  # but we can do a local build here if needed.
  cd "$ROOT_DIR/linggen-cli" && cargo build --release
  cd "$ROOT_DIR/backend" && cargo build --release --bin linggen-server
fi

# 2. Build multi-arch Linux artifacts (requires Docker)
if [ "$SKIP_LINUX" = "true" ]; then
  echo ""
  echo "⏩ Step 2: Skipping multi-arch Linux build."
else
  if command -v docker >/dev/null && docker buildx version >/dev/null 2>&1; then
    echo ""
    echo "🐳 Step 2: Building multi-arch Linux packages via Docker..."
    "$ROOT_DIR/scripts/build-linux.sh"
  else
    echo ""
    echo "⚠️  Docker or Buildx not found. Skipping multi-arch Linux build."
  fi
fi

echo ""
echo "✅ Build complete! All artifacts are in the dist/ directory."
