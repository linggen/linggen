#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CLI_DIR="$ROOT_DIR/linggen-cli"
TARGET_DIR="$CLI_DIR/target/release"

echo "🔨 Building linggen CLI (release)"
cd "$CLI_DIR"
cargo build --release

mkdir -p "$ROOT_DIR/dist"

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

TARBALL="$ROOT_DIR/dist/linggen-cli-${SLUG}.tar.gz"

tar -C "$TARGET_DIR" -czf "$TARBALL" linggen

echo "✅ Built CLI"
echo "Tarball: $TARBALL"
