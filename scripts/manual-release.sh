#!/bin/bash
set -euo pipefail

# Manual release upload script
# Usage: ./scripts/manual-release.sh <version> [--draft]
#        Publishes release by default (required for updater to work)
#        Use --draft to keep as draft (updater won't see it)

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
REPO="linggen/linggen"  # Change to your releases repo

VERSION="${1:-}"
KEEP_DRAFT=false

# Check for --draft flag
if [ "$1" = "--draft" ]; then
  KEEP_DRAFT=true
  VERSION="${2:-}"
fi

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--draft]" >&2
  echo "Example: $0 v0.2.0        # Creates and publishes release (updater can fetch)" >&2
  echo "Example: $0 v0.2.0 --draft  # Creates draft release (updater cannot fetch)" >&2
  exit 1
fi

# Extract version number (remove 'v' prefix)
VERSION_NUM="${VERSION#v}"

echo "🔄 Syncing version $VERSION_NUM to all project files..."
"$ROOT_DIR/scripts/sync-version.sh" "$VERSION_NUM"

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

# Sanity check: ensure the sidecar we ship supports WebUI discovery via env vars.
# If this fails, the release app will fall back to API-only ("Hello from Linggen Backend!").
#
# We intentionally use `grep -a` (treat binary as text) instead of `strings` to avoid
# brittle exit-code behavior under `set -euo pipefail`.
if ! LC_ALL=C grep -a -q "LINGGEN_FRONTEND_DIR" "$ROOT_DIR/backend/target/release/linggen-server"; then
  if [ "${FORCE_CLEAN:-}" = "1" ]; then
    echo "⚠️  Built linggen-server is missing LINGGEN_FRONTEND_DIR support; running cargo clean as requested (FORCE_CLEAN=1)..." >&2
    cargo clean -p api || true
    cargo build --release --bin linggen-server
  fi

  if ! LC_ALL=C grep -a -q "LINGGEN_FRONTEND_DIR" "$ROOT_DIR/backend/target/release/linggen-server"; then
    echo "❌ Built linggen-server is missing LINGGEN_FRONTEND_DIR support." >&2
    echo "   If you recently changed backend code or suspect stale artifacts, run:" >&2
    echo "     (cd \"$ROOT_DIR/backend\" && cargo clean -p api && cargo build --release --bin linggen-server)" >&2
    echo "   Or rerun this script with FORCE_CLEAN=1." >&2
    exit 1
  fi
fi

# Optional: print what we will bundle (useful for debugging target/copy mismatches).
if command -v shasum >/dev/null 2>&1; then
  if [ -f "$ROOT_DIR/backend/target/release/linggen-server" ]; then
    echo "ℹ️  linggen-server (release) sha256: $(shasum -a 256 "$ROOT_DIR/backend/target/release/linggen-server" | awk '{print $1}')"
  else
    echo "ℹ️  linggen-server (release) sha256: (missing file at $ROOT_DIR/backend/target/release/linggen-server)" >&2
  fi
fi

# Step 3: Build Tauri App (macOS only)
APP_PATH=""
APP_TARBALL_NAME=""
UPDATER_TARBALL_NAME=""
UPDATER_SIG_B64=""
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
  # IMPORTANT: Tauri bundles external binaries from `src-tauri/binaries/<name>-<target-triple>`.
  # If we only copy to an ad-hoc filename, Tauri may pick up an older cached sidecar and you
  # end up with a released app that serves API-only at :8787.
  # If you build with a custom target (CARGO_BUILD_TARGET), match that. Otherwise use host triple.
  TARGET_TRIPLE="${CARGO_BUILD_TARGET:-$(rustc -Vv | awk '/^host: /{print $2}')}"
  BIN_DIR="$ROOT_DIR/frontend/src-tauri/binaries"
  mkdir -p "$BIN_DIR"
  SIDECAR_NAME="linggen-server-${TARGET_TRIPLE}"
  # Remove any stale sidecars so Tauri cannot pick the wrong one.
  rm -f "$BIN_DIR/linggen-server-"* 2>/dev/null || true
  rm -f "$BIN_DIR/$SIDECAR_NAME" 2>/dev/null || true

  BACKEND_BIN="$ROOT_DIR/backend/target/release/linggen-server"
  if [ ! -f "$BACKEND_BIN" ]; then
    echo "❌ Backend binary not found at $BACKEND_BIN" >&2
    exit 1
  fi

  cp "$BACKEND_BIN" "$BIN_DIR/$SIDECAR_NAME"
  chmod +x "$BIN_DIR/$SIDECAR_NAME"

  # Back-compat: also place a copy next to tauri.conf.json (older layouts sometimes used this).
  rm -f "linggen-server-${ARCH}-apple-darwin" "linggen-server-${TARGET_TRIPLE}"
  cp "$BACKEND_BIN" "linggen-server-${TARGET_TRIPLE}"
  chmod +x "linggen-server-${TARGET_TRIPLE}"
  
  cd "$ROOT_DIR/frontend"
  # Use npm install if package-lock.json doesn't exist, otherwise use npm ci
  if [ -f "package-lock.json" ]; then
    npm ci
  else
    echo "⚠️  package-lock.json not found, using npm install instead"
    npm install
  fi
  npm run build

  # Make the release app also serve a browser WebUI at http://127.0.0.1:8787/
  # The backend looks for ../Resources/frontend/index.html relative to the sidecar binary.
  # So we must bundle frontend assets into Linggen.app/Contents/Resources/frontend/.
  FRONTEND_DIST_DIR="$ROOT_DIR/frontend/dist"
  FRONTEND_RESOURCE_DIR="$ROOT_DIR/frontend/src-tauri/resources/frontend"
  if [ -d "$FRONTEND_DIST_DIR" ]; then
    rm -rf "$FRONTEND_RESOURCE_DIR"
    mkdir -p "$FRONTEND_RESOURCE_DIR"
    cp -R "$FRONTEND_DIST_DIR/." "$FRONTEND_RESOURCE_DIR/"
    echo "✅ Bundled frontend into resources/frontend (for backend static hosting)"
  else
    echo "⚠️  Frontend dist not found at $FRONTEND_DIST_DIR (skipping resources copy)"
  fi

  # Ensure we run the Tauri build from the Tauri project directory so path resolution is deterministic.
  cd "$ROOT_DIR/frontend/src-tauri"
  
  # Build Tauri app and DMG (keep both)
  # Tauri updater artifacts require the private key at build time to generate *.sig.
  # Provide it via TAURI_SIGNING_PRIVATE_KEY (content string), falling back to unsigned build if missing.
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
    echo "⚠️  No private key found for updater signing (expected at ~/.tauri/linggen.key)."
    echo "   Tauri may fail to generate updater signatures; install-update may not work."
    CI=false cargo tauri build --bundles app
  fi

  # Verify the built app contains the frontend and the *new* sidecar backend.
  # This prevents publishing an app where :8787 falls back to API-only.
  BUILT_APP_PATH="$ROOT_DIR/frontend/src-tauri/target/release/bundle/macos/Linggen.app"
  if [ -d "$BUILT_APP_PATH" ]; then
    if [ ! -f "$BUILT_APP_PATH/Contents/Resources/resources/frontend/index.html" ]; then
      echo "❌ Built app is missing Resources/resources/frontend/index.html" >&2
      echo "   Expected: $BUILT_APP_PATH/Contents/Resources/resources/frontend/index.html" >&2
      exit 1
    fi
    if ! LC_ALL=C grep -a -q "LINGGEN_FRONTEND_DIR" "$BUILT_APP_PATH/Contents/MacOS/linggen-server"; then
      echo "❌ Built app contains an old linggen-server (missing LINGGEN_FRONTEND_DIR support)" >&2
      echo "   Expected: $BUILT_APP_PATH/Contents/MacOS/linggen-server" >&2
      exit 1
    fi
  else
    echo "⚠️  Built app not found at $BUILT_APP_PATH (skipping app-content verification)" >&2
  fi

  # Prefer Tauri-generated updater artifact (*.app.tar.gz) if present.
  # This is the canonical format used by the Tauri updater on macOS.
  # Prefer the deterministic path first to avoid accidentally picking an older artifact.
  UPDATER_BUNDLE_PATH="$ROOT_DIR/frontend/src-tauri/target/release/bundle/macos/Linggen.app.tar.gz"
  if [ ! -f "$UPDATER_BUNDLE_PATH" ]; then
    UPDATER_BUNDLE_PATH=$(find "$ROOT_DIR/frontend/src-tauri/target" -name "*.app.tar.gz" -path "*/bundle/macos/*" 2>/dev/null | head -n 1)
    if [ -z "${UPDATER_BUNDLE_PATH:-}" ]; then
      UPDATER_BUNDLE_PATH=$(find "$ROOT_DIR/frontend/src-tauri/target/release/bundle/macos" -name "*.app.tar.gz" 2>/dev/null | head -n 1)
    fi
  fi

  if [ -n "${UPDATER_BUNDLE_PATH:-}" ] && [ -f "$UPDATER_BUNDLE_PATH" ]; then
    cp "$UPDATER_BUNDLE_PATH" "$DIST_DIR/"
    UPDATER_TARBALL_NAME=$(basename "$UPDATER_BUNDLE_PATH")
    echo "✅ Updater artifact found: dist/${UPDATER_TARBALL_NAME}"

    # Prefer the signature produced by Tauri build (if it exists), otherwise sign ourselves.
    if [ -f "${UPDATER_BUNDLE_PATH}.sig" ]; then
      # Newer Tauri builds already output the signature as a single base64 string in the .sig file.
      # If we base64 it again, it becomes double-encoded and verification will fail.
      UPDATER_SIG_B64=$(tr -d '\n' < "${UPDATER_BUNDLE_PATH}.sig")
      echo "✅ Updater signature found from build"
    else
      echo "  🔐 Signing updater artifact..."
      UPDATER_SIG_B64=$(sign_file "$DIST_DIR/$UPDATER_TARBALL_NAME")
      if [ -n "$UPDATER_SIG_B64" ]; then
        echo "  ✅ Updater artifact signed"
      else
        echo "  ⚠️  Updater artifact signing failed or skipped"
      fi
    fi
  else
    echo "⚠️  Updater artifact (*.app.tar.gz) not found; will fall back to a manually-created .app tarball."
  fi
  
  # Find .app bundle and package tarball for CLI install
  APP_PATH=$(find "$ROOT_DIR/frontend/src-tauri/target" -name "Linggen.app" -path "*/bundle/macos/*" 2>/dev/null | head -n 1)
  if [ -z "$APP_PATH" ]; then
    APP_PATH=$(find "$ROOT_DIR/frontend/src-tauri/target/release/bundle/macos" -name "Linggen.app" 2>/dev/null | head -n 1)
  fi

  # Only build a separate app tarball for the CLI if the updater bundle wasn't produced.
  # When the updater bundle exists, the CLI should reuse the same file as Tauri updater.
  if [ -z "${UPDATER_TARBALL_NAME:-}" ] && [ -n "$APP_PATH" ] && [ -d "$APP_PATH" ]; then
    APP_TARBALL_NAME="linggen-${SLUG}.tar.gz"
    tar -C "$(dirname "$APP_PATH")" -czf "$DIST_DIR/$APP_TARBALL_NAME" "$(basename "$APP_PATH")"
    echo "✅ App tarball built: dist/${APP_TARBALL_NAME}"
  else
    if [ -n "${UPDATER_TARBALL_NAME:-}" ]; then
      echo "ℹ️  Skipping separate app tarball (CLI will reuse updater artifact)"
    else
      echo "⚠️  Linggen.app not found; skipping app tarball."
    fi
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
  echo "✅ Created release ${VERSION} (will be published after assets are uploaded)"
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
    # Read signature if it exists.
    if [ -f "$sig_file" ]; then
      # Tauri may write either:
      # - minisign text format (starts with "untrusted comment:"), OR
      # - a base64-encoded minisign blob (often starts with "dW50..." which decodes to "untrusted comment:")
      #
      # latest.json expects the base64-encoded minisign blob.
      local first_line
      first_line="$(head -n 1 "$sig_file" 2>/dev/null || true)"
      if echo "$first_line" | grep -q '^untrusted comment:'; then
        base64 -i "$sig_file" | tr -d '\n'
      else
        # Already base64; normalize to a single line.
        tr -d '\n' < "$sig_file"
      fi
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

# Upload app tarball (if exists) - legacy path for CLI-driven install.
# Prefer using the updater artifact for both CLI and Tauri when available.
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

# Upload updater tarball (if exists)
if [ -n "$UPDATER_TARBALL_NAME" ] && [ -f "$DIST_DIR/$UPDATER_TARBALL_NAME" ]; then
  echo "  📤 Uploading updater artifact..."
  delete_asset "$UPDATER_TARBALL_NAME"
  gh release upload "$VERSION" "$DIST_DIR/$UPDATER_TARBALL_NAME" --repo "$REPO"
fi

# Step 6: Generate and upload manifests
echo ""
echo "6️⃣  Generating manifests..."

BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

# Generate manifest.json (valid JSON; include universal keys for CLI compatibility)
if [ "$OS" = "darwin" ]; then
  optional_app_tarball=""
  
  # Build CLI entry with signature if available
  CLI_SIG_JSON=""
  if [ -n "$CLI_SIG" ]; then
    CLI_SIG_JSON=", \"signature\": \"${CLI_SIG}\""
  fi

  # App artifact for macOS:
  # Prefer the updater artifact (same as Tauri uses), otherwise fall back to a manually-created tarball.
  if [ -n "$UPDATER_TARBALL_NAME" ] && [ -n "$UPDATER_SIG_B64" ]; then
    optional_app_tarball=$(cat <<EOF_APP
,
    "app-macos-tarball": {"url": "${BASE_URL}/${UPDATER_TARBALL_NAME}", "signature": "${UPDATER_SIG_B64}"}
EOF_APP
)
  elif [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
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

  cat > "$DIST_DIR/manifest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "artifacts": {
    "cli-macos-universal": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}},
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}}${optional_app_tarball}
  }
}
EOF
else
  CLI_SIG_JSON=""
  if [ -n "$CLI_SIG" ]; then
    CLI_SIG_JSON=", \"signature\": \"${CLI_SIG}\""
  fi

  cat > "$DIST_DIR/manifest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "artifacts": {
    "cli-${SLUG}": {"url": "${BASE_URL}/linggen-cli-${SLUG}.tar.gz"${CLI_SIG_JSON}}
  }
}
EOF
fi

# Generate latest.json (Tauri updater artifact: *.app.tar.gz)
# On macOS, Tauri updater expects the .app.tar.gz bundle, NOT the .dmg.
LATEST_TARBALL_NAME="${UPDATER_TARBALL_NAME:-}"
LATEST_SIG_B64="${UPDATER_SIG_B64:-}"
if [ -z "$LATEST_TARBALL_NAME" ] && [ -n "$APP_TARBALL_NAME" ] && [ -f "$DIST_DIR/$APP_TARBALL_NAME" ]; then
  # Fallback: use manually-created tarball of Linggen.app (less ideal than *.app.tar.gz)
  LATEST_TARBALL_NAME="$APP_TARBALL_NAME"
  LATEST_SIG_B64="$APP_SIG"
fi

if [ -n "$LATEST_TARBALL_NAME" ] && [ -f "$DIST_DIR/$LATEST_TARBALL_NAME" ]; then
  # Determine Tauri platform key based on SLUG
  TAURI_PLATFORM="darwin-aarch64"
  if [[ "$SLUG" == *"x86_64"* ]]; then
    TAURI_PLATFORM="darwin-x86_64"
  elif [[ "$SLUG" == *"aarch64"* ]]; then
    TAURI_PLATFORM="darwin-aarch64"
  fi

  # Generate latest.json with properly escaped signature
  # Use jq to ensure proper JSON escaping of the signature string
  # Tauri expects a base64-encoded minisign signature (generated by `tauri signer sign`)
  if [ -z "$LATEST_SIG_B64" ]; then
    echo "❌ Updater signature is missing for $LATEST_TARBALL_NAME"
    echo "   Make sure TAURI_PRIVATE_KEY / ~/.tauri/linggen.key and password are configured."
    exit 1
  fi

  if command -v jq >/dev/null 2>&1; then
    # Verify signature format before generating JSON
    echo "  📝 Updater signature preview (base64, first 50 chars): $(echo "$LATEST_SIG_B64" | head -c 50)..."
    
    jq -n \
      --arg version "$VERSION_NUM" \
      --arg notes "See release notes at https://github.com/${REPO}/releases/tag/${VERSION}" \
      --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
      --arg platform "${TAURI_PLATFORM}" \
      --arg signature "$LATEST_SIG_B64" \
      --arg url "${BASE_URL}/${LATEST_TARBALL_NAME}" \
      '{
        version: $version,
        notes: $notes,
        pub_date: $pub_date,
        platforms: {
          ($platform): {
            signature: $signature,
            url: $url
          }
        }
      }' > "$DIST_DIR/latest.json"
    
    # Verify the generated JSON has the signature properly formatted
    json_sig=$(jq -r ".platforms.\"${TAURI_PLATFORM}\".signature" "$DIST_DIR/latest.json")
    if [ -n "$json_sig" ]; then
      echo "  ✅ Generated JSON includes updater signature"
    else
      echo "  ❌ Generated JSON is missing updater signature"
      exit 1
    fi
  else
    # Fallback: manual JSON generation (less safe but works)
    cat > "$DIST_DIR/latest.json" << EOF
{
  "version": "${VERSION_NUM}",
  "notes": "See release notes at https://github.com/${REPO}/releases/tag/${VERSION}",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "${TAURI_PLATFORM}": {
      "signature": "${LATEST_SIG_B64}",
      "url": "${BASE_URL}/${LATEST_TARBALL_NAME}"
    }
  }
}
EOF
  fi
  delete_asset "latest.json"
  gh release upload "$VERSION" "$DIST_DIR/latest.json" --repo "$REPO"
  
  # Note: The updater signature is embedded in latest.json.
fi

delete_asset "manifest.json"
gh release upload "$VERSION" "$DIST_DIR/manifest.json" --repo "$REPO"

# Step 7: Publish release (unless --draft flag was used)
if [ "$KEEP_DRAFT" = "true" ]; then
  echo ""
  echo "⚠️  Draft release ${VERSION} created (NOT published)"
  echo "   ⚠️  WARNING: Updater cannot fetch draft releases!"
  echo "   To publish: gh release edit ${VERSION} --draft=false --repo ${REPO}"
else
  echo ""
  echo "7️⃣  Publishing release..."
  gh release edit "$VERSION" --draft=false --latest --repo "$REPO"
  echo ""
  echo "✅ Release ${VERSION} published and marked as latest!"
  echo "   Updater can now fetch this release"
fi
echo ""
echo "📋 Uploaded artifacts:"
echo "  - linggen-cli-${SLUG}.tar.gz"
[ -n "$APP_TARBALL_NAME" ] && echo "  - ${APP_TARBALL_NAME} (legacy; CLI app install only)"
[ -n "${UPDATER_TARBALL_NAME:-}" ] && echo "  - ${UPDATER_TARBALL_NAME} (updater; used by Tauri + CLI)"
echo "  - manifest.json"
[ -n "${UPDATER_TARBALL_NAME:-}" ] && echo "  - latest.json (contains embedded signature)"
echo ""
echo "📥 Install CLI:"
echo "   curl -fsSL https://linggen.dev/install-cli.sh | bash"
echo ""
echo "📥 Install App:"
echo "   linggen install"
