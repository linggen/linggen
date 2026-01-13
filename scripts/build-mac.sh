#!/bin/bash
set -euo pipefail

# Packaging script for Linggen
# Usage: ./scripts/package.sh <version>
# This script builds and signs all artifacts but does NOT upload them.

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

echo "🔄 Syncing version $VERSION_NUM to all project files..."
"$ROOT_DIR/scripts/sync-version.sh" "$VERSION_NUM"

SLUG=$(detect_platform)
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

echo "🚀 Packaging Linggen ${VERSION} for ${SLUG}"
echo "=========================================="

# Step 1: Build CLI
echo "1️⃣  Building CLI..."
cd "$ROOT_DIR/linggen-cli"
cargo build --release
tar -C target/release -czf "$DIST_DIR/linggen-cli-${SLUG}.tar.gz" linggen
echo "✅ CLI built: dist/linggen-cli-${SLUG}.tar.gz"

# Step 2: Build Backend
echo ""
echo "2️⃣  Building Server..."
cd "$ROOT_DIR/backend"
cargo build --release --bin linggen-server
echo "✅ Backend built"

# Verify LINGGEN_FRONTEND_DIR support
if ! LC_ALL=C grep -a -q "LINGGEN_FRONTEND_DIR" "$ROOT_DIR/backend/target/release/linggen-server"; then
  echo "❌ Built linggen-server is missing LINGGEN_FRONTEND_DIR support." >&2
  exit 1
fi

# Step 3: Build Tauri App (macOS only for now)
if [ "$OS" = "darwin" ]; then
  echo ""
  echo "3️⃣  Building Tauri App..."
  
  # Prepare sidecar
  TARGET_TRIPLE="${CARGO_BUILD_TARGET:-$(rustc -Vv | awk '/^host: /{print $2}')}"
  BIN_DIR="$ROOT_DIR/frontend/src-tauri/binaries"
  mkdir -p "$BIN_DIR"
  SIDECAR_NAME="linggen-server-${TARGET_TRIPLE}"
  rm -f "$BIN_DIR/linggen-server-"* 2>/dev/null || true
  cp "$ROOT_DIR/backend/target/release/linggen-server" "$BIN_DIR/$SIDECAR_NAME"
  chmod +x "$BIN_DIR/$SIDECAR_NAME"

  # Build frontend
  cd "$ROOT_DIR/frontend"
  if [ -f "package-lock.json" ]; then npm ci; else npm install; fi
  npm run build

  # Bundle frontend into resources
  FRONTEND_DIST_DIR="$ROOT_DIR/frontend/dist"
  FRONTEND_RESOURCE_DIR="$ROOT_DIR/frontend/src-tauri/resources/frontend"
  rm -rf "$FRONTEND_RESOURCE_DIR"
  mkdir -p "$FRONTEND_RESOURCE_DIR"
  cp -R "$FRONTEND_DIST_DIR/." "$FRONTEND_RESOURCE_DIR/"

  # Build Tauri app
  cd "$ROOT_DIR/frontend/src-tauri"
  
  # Ensure signing keys are available for the build
  TAURI_BUILD_PASSWORD=""
  CONFIG_FILE="$ROOT_DIR/.tauri-signing.conf"
  if [ -f "$CONFIG_FILE" ]; then
    TAURI_BUILD_PASSWORD=$(grep -E "^TAURI_PRIVATE_KEY_PASSWORD=" "$CONFIG_FILE" | grep -v "^#" | cut -d'=' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | tr -d '"' | tr -d "'")
  fi
  TAURI_BUILD_PASSWORD="${TAURI_BUILD_PASSWORD:-${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-${TAURI_PRIVATE_KEY_PASSWORD:-}}}"

  TAURI_BUILD_KEY_CONTENT=""
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    TAURI_BUILD_KEY_CONTENT="$TAURI_SIGNING_PRIVATE_KEY"
  elif [ -n "${TAURI_PRIVATE_KEY:-}" ]; then
    TAURI_BUILD_KEY_CONTENT="$TAURI_PRIVATE_KEY"
  elif [ -f "$HOME/.tauri/linggen.key" ]; then
    TAURI_BUILD_KEY_CONTENT="$(tr -d '\n' < "$HOME/.tauri/linggen.key")"
  fi

  if [ -n "$TAURI_BUILD_KEY_CONTENT" ]; then
    TAURI_SIGNING_PRIVATE_KEY="$TAURI_BUILD_KEY_CONTENT" \
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$TAURI_BUILD_PASSWORD" \
    CI=false cargo tauri build --bundles app
  else
    CI=false cargo tauri build --bundles app
  fi

  # Copy artifacts to dist
  UPDATER_BUNDLE_PATH="$ROOT_DIR/frontend/src-tauri/target/release/bundle/macos/Linggen.app.tar.gz"
  if [ -f "$UPDATER_BUNDLE_PATH" ]; then
    cp "$UPDATER_BUNDLE_PATH" "$DIST_DIR/"
    if [ -f "${UPDATER_BUNDLE_PATH}.sig" ]; then
      cp "${UPDATER_BUNDLE_PATH}.sig" "$DIST_DIR/$(basename "$UPDATER_BUNDLE_PATH").sig"
    fi
  fi
fi

# Step 4: Signing
echo ""
echo "4️⃣  Signing Artifacts..."
CLI_TARBALL="$DIST_DIR/linggen-cli-${SLUG}.tar.gz"
echo "  🔐 Signing CLI tarball..."
CLI_SIG=$(sign_file "$CLI_TARBALL" "$ROOT_DIR") || true
if [ -n "$CLI_SIG" ]; then
  echo "$CLI_SIG" > "${CLI_TARBALL}.sig.txt"
  echo "  ✅ CLI tarball signed"
fi

# macOS specific signing for updater
if [ "$OS" = "darwin" ]; then
  UPDATER_TARBALL="$DIST_DIR/Linggen.app.tar.gz"
  if [ -f "$UPDATER_TARBALL" ]; then
    # If Tauri didn't produce a .sig file, we generate it
    if [ ! -f "${UPDATER_TARBALL}.sig" ]; then
      echo "  🔐 Signing updater tarball..."
      UPDATER_SIG=$(sign_file "$UPDATER_TARBALL" "$ROOT_DIR") || true
      if [ -n "$UPDATER_SIG" ]; then
        echo "$UPDATER_SIG" > "${UPDATER_TARBALL}.sig.txt"
        echo "  ✅ Updater tarball signed"
      fi
    else
      # Move Tauri's sig to our text format for easier reading by release script
      tr -d '\n' < "${UPDATER_TARBALL}.sig" > "${UPDATER_TARBALL}.sig.txt"
    fi
  fi
fi

echo ""
echo "✅ Packaging complete. Artifacts are in $DIST_DIR"
