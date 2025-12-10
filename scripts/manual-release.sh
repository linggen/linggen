#!/bin/bash
set -euo pipefail

# Manual release upload script
# Usage: ./scripts/manual-release.sh v0.2.0

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>"
  echo "Example: $0 v0.2.0"
  exit 1
fi

VERSION_NUM="${VERSION#v}"
REPO="linggen/linggen-releases"  # Change to your releases repo
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"

echo "🚀 Manual Release Upload for ${VERSION}"
echo "=========================================="

# Detect platform
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

echo "📦 Platform: ${SLUG}"
echo ""

# Step 1: Build CLI
echo "1️⃣  Building CLI..."
cd "$ROOT_DIR/linggen-cli"
cargo build --release
mkdir -p "$DIST_DIR"
tar -C target/release -czf "$DIST_DIR/linggen-cli-${SLUG}.tar.gz" linggen
echo "✅ CLI built: dist/linggen-cli-${SLUG}.tar.gz"

# Step 2: Build Server (for Tauri sidecar only)
echo ""
echo "2️⃣  Building Server..."
cd "$ROOT_DIR/backend"
cargo build --release --bin linggen-server
echo "✅ Server built (for Tauri sidecar)"

# Step 3: Build Tauri App (macOS only)
DMG_PATH=""
DMG_NAME=""
APP_PATH=""
APP_TARBALL_NAME=""
if [ "$OS" = "darwin" ]; then
  echo ""
  echo "3️⃣  Building Tauri App..."
  
  # Install Tauri CLI if not available
  if ! command -v cargo-tauri &> /dev/null && ! cargo tauri --version &> /dev/null; then
    echo "  Installing Tauri CLI..."
    cargo install tauri-cli --locked
  fi
  
  cd "$ROOT_DIR/frontend/src-tauri"
  
  # Copy server as sidecar
  SIDECAR_NAME="linggen-server-${ARCH}-apple-darwin"
  cp "$ROOT_DIR/backend/target/release/linggen-server" "$SIDECAR_NAME"
  chmod +x "$SIDECAR_NAME"
  
  cd "$ROOT_DIR/frontend"
  npm ci
  npm run build
  
  # Build Tauri app and DMG (keep both)
  cargo tauri build --bundles app,dmg
  
  # Find DMG - check both possible paths
  DMG_PATH=$(find src-tauri/target -name "*.dmg" -path "*/bundle/dmg/*" | head -n 1)
  if [ -z "$DMG_PATH" ]; then
    # Try alternative path structure
    DMG_PATH=$(find src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -n 1)
  fi
  
  if [ -n "$DMG_PATH" ] && [ -f "$DMG_PATH" ]; then
    cp "$DMG_PATH" "$DIST_DIR/"
    DMG_NAME=$(basename "$DMG_PATH")
    echo "✅ DMG built: dist/${DMG_NAME}"
  else
    echo "⚠️  DMG not found. Searched in:"
    echo "   - src-tauri/target/*/release/bundle/dmg"
    echo "   - src-tauri/target/release/bundle/dmg"
    ls -la src-tauri/target/release/bundle/dmg/ 2>/dev/null || echo "   Directory does not exist"
  fi

  # Find .app bundle and package tarball for CLI install
  APP_PATH=$(find src-tauri/target -name "Linggen.app" -path "*/bundle/macos/*" | head -n 1)
  if [ -z "$APP_PATH" ]; then
    APP_PATH=$(find src-tauri/target/release/bundle/macos -name "Linggen.app" 2>/dev/null | head -n 1)
  fi

  if [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
    APP_TARBALL_NAME="linggen-${SLUG}.tar.gz"
    tar -C "$(dirname "$APP_PATH")" -czf "$DIST_DIR/$APP_TARBALL_NAME" "$(basename "$APP_PATH")"
    echo "✅ App tarball built: dist/${APP_TARBALL_NAME}"
  else
    echo "⚠️  Linggen.app not found; skipping app tarball."
  fi
fi

# Step 4: Create release (if it doesn't exist)
echo ""
echo "4️⃣  Creating GitHub Release..."
if gh release view "$VERSION" --repo "$REPO" &>/dev/null; then
  echo "✅ Release ${VERSION} already exists"
else
  gh release create "$VERSION" \
    --repo "$REPO" \
    --title "Linggen ${VERSION}" \
    --notes "Release ${VERSION} - Manual upload" \
    --draft
  echo "✅ Created draft release ${VERSION}"
fi

# Helper to replace assets if they already exist
delete_asset() {
  local name="$1"
  gh release delete-asset "$VERSION" "$name" --repo "$REPO" --yes 2>/dev/null || true
}

# Step 5: Upload artifacts
echo ""
echo "5️⃣  Uploading artifacts..."

# Upload CLI (base name only)
CLI_TARBALL="$DIST_DIR/linggen-cli-${SLUG}.tar.gz"
if [ -f "$CLI_TARBALL" ]; then
  echo "  📤 Uploading CLI base name..."
  delete_asset "linggen-cli-${SLUG}.tar.gz"
  gh release upload "$VERSION" "$CLI_TARBALL" --repo "$REPO"
fi

# Upload DMG (if exists)
if [ -n "$DMG_PATH" ] && [ -f "$DIST_DIR/$DMG_NAME" ]; then
  echo "  📤 Uploading DMG..."
  delete_asset "$DMG_NAME"
  gh release upload "$VERSION" "$DIST_DIR/$DMG_NAME" --repo "$REPO"
fi

# Upload app tarball (if exists)
if [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
  echo "  📤 Uploading app tarball..."
  delete_asset "$APP_TARBALL_NAME"
  gh release upload "$VERSION" "$DIST_DIR/$APP_TARBALL_NAME" --repo "$REPO"
fi

# Step 6: Generate and upload manifests
echo ""
echo "6️⃣  Generating manifests..."

BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

# Generate manifest.json (valid JSON; include universal keys for CLI compatibility)
if [ "$OS" = "darwin" ]; then
  optional_app_dmg=""
  optional_app_tarball=""

  if [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
    optional_app_tarball=$(cat <<EOF_APP
,
    "app-macos-tarball": {"url": "${BASE_URL}/${APP_TARBALL_NAME}"}
EOF_APP
)
  fi

  if [ -n "$DMG_NAME" ]; then
    optional_app_dmg=$(cat <<EOF_APP
,
    "app-macos-dmg": {"url": "${BASE_URL}/${DMG_NAME}"}
EOF_APP
)
  fi

  cat > "$DIST_DIR/manifest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "artifacts": {
    "cli-macos-universal": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"},
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"}${optional_app_tarball}${optional_app_dmg}
  }
}
EOF
else
  optional_app_dmg=""
  if [ -n "$DMG_NAME" ]; then
    optional_app_dmg=$(cat <<EOF_APP
,
    "app-macos-dmg": {"url": "${BASE_URL}/${DMG_NAME}"}
EOF_APP
)
  fi

  cat > "$DIST_DIR/manifest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "artifacts": {
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"}${optional_app_dmg}
  }
}
EOF
fi

# Generate latest.json (if DMG exists)
if [ -n "$DMG_NAME" ]; then
  cat > "$DIST_DIR/latest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "notes": "See release notes at https://github.com/${REPO}/releases/tag/${VERSION}",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "darwin-universal": {
      "signature": "",
      "url": "${BASE_URL}/${DMG_NAME}"
    }
  }
}
EOF
  delete_asset "latest.json"
  gh release upload "$VERSION" "$DIST_DIR/latest.json" --repo "$REPO"
fi

delete_asset "manifest.json"
gh release upload "$VERSION" "$DIST_DIR/manifest.json" --repo "$REPO"

# Step 7: Publish release
echo ""
echo "7️⃣  Publishing release..."
gh release edit "$VERSION" --draft=false --repo "$REPO"

echo ""
echo "✅ Release ${VERSION} published successfully!"
echo ""
echo "📋 Uploaded artifacts:"
echo "  - linggen-cli-${SLUG}.tar.gz"
[ -n "$APP_TARBALL_NAME" ] && echo "  - ${APP_TARBALL_NAME}"
[ -n "$DMG_NAME" ] && echo "  - ${DMG_NAME}"
echo "  - manifest.json"
[ -n "$DMG_NAME" ] && echo "  - latest.json"
echo " curl -fsSL https://linggen.dev/install-cli.sh | bash "
