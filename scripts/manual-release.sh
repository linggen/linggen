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
DMG_SIG=""
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
    
    # Sign the DMG
    echo "🔐 Signing DMG..."
    
    # Read password from config file or environment variable
    DMG_PASSWORD=""
    CONFIG_FILE="$ROOT_DIR/.tauri-signing.conf"
    if [ -f "$CONFIG_FILE" ]; then
      DMG_PASSWORD=$(grep -E "^TAURI_PRIVATE_KEY_PASSWORD=" "$CONFIG_FILE" | grep -v "^#" | cut -d'=' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | tr -d '"' | tr -d "'")
    fi
    DMG_PASSWORD="${DMG_PASSWORD:-${TAURI_PRIVATE_KEY_PASSWORD:-}}"
    
    # Read key content
    DMG_KEY_CONTENT=""
    if [ -n "${TAURI_PRIVATE_KEY:-}" ]; then
      DMG_KEY_CONTENT="$TAURI_PRIVATE_KEY"
    elif [ -f "$HOME/.tauri/linggen.key" ]; then
      DMG_KEY_CONTENT="$(cat "$HOME/.tauri/linggen.key")"
    else
      echo "⚠️  No signing key found. Set TAURI_PRIVATE_KEY or create ~/.tauri/linggen.key"
      echo "   Generate key with: tauri signer generate --write-keys"
      echo "   DMG will be unsigned (updater may reject it)"
    fi
    
    # Sign using key content as string (not file path)
    if [ -n "$DMG_KEY_CONTENT" ]; then
      if npx tauri signer sign -k "$DMG_KEY_CONTENT" -p "$DMG_PASSWORD" "$DIST_DIR/$DMG_NAME" >/dev/null 2>&1; then
        echo "✅ DMG signed"
      else
        echo "⚠️  Signing failed, continuing without signature"
      fi
    fi
    
    # Read signature if it exists
    if [ -f "$DIST_DIR/${DMG_NAME}.sig" ]; then
      DMG_SIG=$(cat "$DIST_DIR/${DMG_NAME}.sig")
      echo "✅ DMG signed"
    else
      DMG_SIG=""
      echo "⚠️  DMG signature not found; latest.json will have empty signature"
    fi
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

# Helper to sign a file and return the signature (base64 encoded full signature)
sign_file() {
  local file_path="$1"
  local sig_file="${file_path}.sig"
  
  # Read password from config file or environment variable
  local password=""
  local config_file="$ROOT_DIR/.tauri-signing.conf"
  if [ -f "$config_file" ]; then
    # Read password from config file (strip quotes and comments)
    password=$(grep -E "^TAURI_PRIVATE_KEY_PASSWORD=" "$config_file" | grep -v "^#" | cut -d'=' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | tr -d '"' | tr -d "'")
  fi
  
  # Fallback to environment variable, then empty string
  password="${password:-${TAURI_PRIVATE_KEY_PASSWORD:-}}"
  
  # Read key content and sign using -k (string) option
  # This works better than -f (file path) which has format issues
  if [ -n "${TAURI_PRIVATE_KEY:-}" ]; then
    # Use environment variable if set
    KEY_CONTENT="$TAURI_PRIVATE_KEY"
  elif [ -f "$HOME/.tauri/linggen.key" ]; then
    # Read key file content
    KEY_CONTENT="$(cat "$HOME/.tauri/linggen.key")"
  else
    echo "⚠️  No signing key found. Set TAURI_PRIVATE_KEY or create ~/.tauri/linggen.key" >&2
    echo ""  # Return empty on failure
    return 1
  fi
  
  # Sign using key content as string (not file path)
  if npx tauri signer sign -k "$KEY_CONTENT" -p "$password" "$file_path" >/dev/null 2>&1; then
    # Read signature if it exists and return base64 encoded full signature file
    if [ -f "$sig_file" ]; then
      # The signature file contains the full minisign signature format
      # We'll base64 encode the entire file content for storage in manifest
      base64 -i "$sig_file" | tr -d '\n'
    else
      echo ""  # No signature file created
      return 1
    fi
  else
    echo "⚠️  Signing failed. File will be unsigned." >&2
    echo ""  # Return empty on failure
    return 1
  fi
}

# Step 5: Upload artifacts
echo ""
echo "5️⃣  Uploading artifacts..."

# Upload CLI (base name only)
CLI_TARBALL="$DIST_DIR/linggen-cli-${SLUG}.tar.gz"
CLI_SIG=""
if [ -f "$CLI_TARBALL" ]; then
  echo "  📤 Uploading CLI base name..."
  delete_asset "linggen-cli-${SLUG}.tar.gz"
  gh release upload "$VERSION" "$CLI_TARBALL" --repo "$REPO"
  
  # Sign the CLI tarball
  echo "  🔐 Signing CLI tarball..."
  CLI_SIG=$(sign_file "$CLI_TARBALL")
  if [ -n "$CLI_SIG" ]; then
    echo "  ✅ CLI tarball signed"
  else
    echo "  ⚠️  CLI tarball signing failed or skipped"
  fi
fi

# Upload DMG (if exists)
if [ -n "$DMG_PATH" ] && [ -f "$DIST_DIR/$DMG_NAME" ]; then
  echo "  📤 Uploading DMG..."
  delete_asset "$DMG_NAME"
  gh release upload "$VERSION" "$DIST_DIR/$DMG_NAME" --repo "$REPO"
fi

# Upload app tarball (if exists)
APP_SIG=""
if [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
  echo "  📤 Uploading app tarball..."
  delete_asset "$APP_TARBALL_NAME"
  gh release upload "$VERSION" "$DIST_DIR/$APP_TARBALL_NAME" --repo "$REPO"
  
  # Sign the app tarball
  echo "  🔐 Signing app tarball..."
  APP_SIG=$(sign_file "$DIST_DIR/$APP_TARBALL_NAME")
  if [ -n "$APP_SIG" ]; then
    echo "  ✅ App tarball signed"
  else
    echo "  ⚠️  App tarball signing failed or skipped"
  fi
fi

# Step 6: Generate and upload manifests
echo ""
echo "6️⃣  Generating manifests..."

BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

# Generate manifest.json (valid JSON; include universal keys for CLI compatibility)
if [ "$OS" = "darwin" ]; then
  optional_app_dmg=""
  optional_app_tarball=""
  
  # Build CLI entry with signature if available
  CLI_SIG_JSON=""
  if [ -n "$CLI_SIG" ]; then
    CLI_SIG_JSON=", \"signature\": \"${CLI_SIG}\""
  fi

  if [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
    APP_SIG_JSON=""
    if [ -n "$APP_SIG" ]; then
      APP_SIG_JSON=", \"signature\": \"${APP_SIG}\""
    fi
    optional_app_tarball=$(cat <<EOF_APP
,
    "app-macos-tarball": {"url": "${BASE_URL}/${APP_TARBALL_NAME}"${APP_SIG_JSON}}
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
    "cli-macos-universal": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}},
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}}${optional_app_tarball}${optional_app_dmg}
  }
}
EOF
else
  optional_app_dmg=""
  CLI_SIG_JSON=""
  if [ -n "$CLI_SIG" ]; then
    CLI_SIG_JSON=", \"signature\": \"${CLI_SIG}\""
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
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}}${optional_app_dmg}
  }
}
EOF
fi

# Generate latest.json (if DMG exists)
if [ -n "$DMG_NAME" ]; then
  # Determine Tauri platform key based on SLUG
  TAURI_PLATFORM="darwin-aarch64"
  if [[ "$SLUG" == *"x86_64"* ]]; then
    TAURI_PLATFORM="darwin-x86_64"
  elif [[ "$SLUG" == *"aarch64"* ]]; then
    TAURI_PLATFORM="darwin-aarch64"
  fi

  cat > "$DIST_DIR/latest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "notes": "See release notes at https://github.com/${REPO}/releases/tag/${VERSION}",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "${TAURI_PLATFORM}": {
      "signature": "${DMG_SIG}",
      "url": "${BASE_URL}/${DMG_NAME}"
    }
  }
}
EOF
  delete_asset "latest.json"
  gh release upload "$VERSION" "$DIST_DIR/latest.json" --repo "$REPO"
  
  # Upload signature file if it exists
  if [ -f "$DIST_DIR/${DMG_NAME}.sig" ]; then
    delete_asset "${DMG_NAME}.sig"
    gh release upload "$VERSION" "$DIST_DIR/${DMG_NAME}.sig" --repo "$REPO"
  fi
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
[ -n "$DMG_SIG" ] && echo "  - ${DMG_NAME}.sig (signature)"
echo "  - manifest.json"
[ -n "$DMG_NAME" ] && echo "  - latest.json"
echo ""
echo "📥 Install CLI:"
echo "   curl -fsSL https://linggen.dev/install-cli.sh | bash"
echo ""
echo "📥 Install App:"
echo "   linggen install"
