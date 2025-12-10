#!/bin/bash
set -euo pipefail

# Simple helper to build and package the CLI for local testing.
# Usage: scripts/publish-cli-local.sh v0.2.0 /tmp/out
# - Builds linggen CLI (release)
# - Produces dist/linggen-cli-<target>.tar.gz
# - Copies tarball to DEST if provided
# - Emits a sample manifest.json you can host locally and point LINGGEN_MANIFEST_URL to

VERSION="${1:-v0.0.0-local}"
DEST="${2:-}"

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLI_DIR="$ROOT_DIR/linggen-cli"
TARGET_DIR="$CLI_DIR/target/release"
DIST_DIR="$ROOT_DIR/dist"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
  darwin)
    case "$ARCH" in
      arm64|aarch64) SLUG="macos-aarch64" ;;
      x86_64|amd64)  SLUG="macos-x86_64" ;;
      *) echo "Unsupported macOS arch: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  linux)
    case "$ARCH" in
      x86_64|amd64) SLUG="linux-x86_64" ;;
      arm64|aarch64) SLUG="linux-arm64" ;;
      *) echo "Unsupported Linux arch: $ARCH" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Unsupported OS: $OS" >&2; exit 1
    ;;
esac

TARBALL_NAME="linggen-cli-${SLUG}.tar.gz"

mkdir -p "$DIST_DIR"

echo "🔨 Building linggen CLI (release)"
cd "$CLI_DIR"
cargo build --release

echo "📦 Packaging tarball"
tar -C "$TARGET_DIR" -czf "$DIST_DIR/$TARBALL_NAME" linggen

# Create a minimal manifest pointing to the tarball path/URL (local path by default)
MANIFEST="$DIST_DIR/manifest.json"
TARBALL_URL="${DEST:-$DIST_DIR}/$TARBALL_NAME"
cat > "$MANIFEST" <<EOF
{
  "version": "${VERSION#v}",
  "artifacts": {
    "cli-${SLUG}": { "url": "$TARBALL_URL" }
  }
}
EOF

echo "✅ Done"
echo "Tarball: $DIST_DIR/$TARBALL_NAME"
echo "Manifest: $MANIFEST"

echo "To test install/update with local manifest:"
echo "  export LINGGEN_MANIFEST_URL=file://$MANIFEST"
echo "  linggen install"

echo "If you set DEST to a web URL or mounted path, update the URL accordingly."
