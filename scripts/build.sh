#!/bin/bash
set -euo pipefail

# Master build orchestrator script for Linggen Agent
# Usage: ./scripts/build.sh <version> [--platform mac|linux]
#
# Default platform is the current host (no cross-build):
#   - macOS host  → mac
#   - Linux host  → linux

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

VERSION=""
PLATFORM=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform)
      PLATFORM="${2:-}"; shift 2 ;;
    --platform=*)
      PLATFORM="${1#--platform=}"; shift ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      fi
      shift ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--platform mac|linux]" >&2
  exit 1
fi

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
HOST_PLATFORM="$([ "$OS" = "darwin" ] && echo mac || echo linux)"
PLATFORM="${PLATFORM:-$HOST_PLATFORM}"

case "$PLATFORM" in
  mac|linux) ;;
  *)
    echo "Error: --platform must be 'mac' or 'linux' (got '$PLATFORM')" >&2
    exit 1 ;;
esac

VERSION_NUM="${VERSION#v}"

echo "🏗️  Building Linggen ${VERSION} (platform: ${PLATFORM})"
echo "=============================="

# Fail here, before any work, if the toolchain isn't reachable. Exported PATH
# carries into sync-version.sh and build-mac.sh.
ensure_cargo

# 0. Sync version to all project files
echo "🔄 Syncing version $VERSION_NUM to all project files..."
"$ROOT_DIR/scripts/sync-version.sh" "$VERSION_NUM"

# 0.5 Clean dist/ to avoid uploading stale artifacts if a later step doesn't produce a new file.
echo "🧹 Cleaning dist/..."
rm -rf "$ROOT_DIR/dist"
mkdir -p "$ROOT_DIR/dist"

# 1. Build artifacts for the selected platform
if [ "$PLATFORM" = "mac" ]; then
  if [ "$OS" != "darwin" ]; then
    echo "Error: --platform mac requires a macOS host" >&2
    exit 1
  fi
  echo "📦 Building macOS artifacts..."
  "$ROOT_DIR/scripts/build-mac.sh" "$VERSION"
else
  if ! (command -v docker >/dev/null && docker buildx version >/dev/null 2>&1); then
    echo "Error: --platform linux requires Docker + buildx" >&2
    exit 1
  fi
  echo "🐳 Building multi-arch Linux packages via Docker..."
  "$ROOT_DIR/scripts/build-linux.sh" "$VERSION"
fi

echo ""
echo "✅ Build complete! All artifacts are in the dist/ directory."
