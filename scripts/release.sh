#!/bin/bash
set -euo pipefail

# Release orchestrator script for Linggen
# Usage: ./scripts/release.sh <version> [--draft] [--skip-linux]

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

REPO="linggen/linggen"
VERSION=""
KEEP_DRAFT=false
PASS_ARGS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --draft)
      KEEP_DRAFT=true
      shift ;;
    --skip-linux)
      PASS_ARGS+=("--skip-linux")
      shift ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      fi
      shift ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--draft] [--skip-linux]" >&2
  exit 1
fi

VERSION_NUM="${VERSION#v}"
DIST_DIR="$ROOT_DIR/dist"

# Step 1: Build everything
echo "📦 Step 1: Building all artifacts..."
"$ROOT_DIR/scripts/build.sh" "$VERSION" ${PASS_ARGS[@]+"${PASS_ARGS[@]}"}

SLUG=$(detect_platform)
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"

# Step 2: Create GitHub Release
echo ""
echo "🚀 Step 2: Creating GitHub Release..."
if gh release view "$VERSION" --repo "$REPO" &>/dev/null; then
  echo "✅ Release ${VERSION} already exists"
else
  gh release create "$VERSION" \
    --repo "$REPO" \
    --title "Linggen ${VERSION}" \
    --notes "Release ${VERSION} - Automated upload" \
    --draft
  echo "✅ Created draft release ${VERSION}"
fi

# Step 3: Upload Artifacts
echo ""
echo "📤 Step 3: Uploading artifacts..."

delete_asset() {
  local name="$1"
  gh release delete-asset "$VERSION" "$name" --repo "$REPO" --yes 2>/dev/null || true
}

# CLI Tarball (Local Platform)
CLI_TARBALL="$DIST_DIR/linggen-cli-${SLUG}.tar.gz"
if [ -f "$CLI_TARBALL" ]; then
  echo "  Uploading CLI: $(basename "$CLI_TARBALL")"
  delete_asset "$(basename "$CLI_TARBALL")"
  gh release upload "$VERSION" "$CLI_TARBALL" --repo "$REPO"
fi

# Updater Tarball (macOS)
if [ "$OS" = "darwin" ]; then
  UPDATER_TARBALL="$DIST_DIR/Linggen.app.tar.gz"
  if [ -f "$UPDATER_TARBALL" ]; then
    echo "  Uploading Updater: $(basename "$UPDATER_TARBALL")"
    delete_asset "$(basename "$UPDATER_TARBALL")"
    gh release upload "$VERSION" "$UPDATER_TARBALL" --repo "$REPO"
  fi
fi

# Linux Artifacts (Multi-Arch from Docker)
if [ -d "$DIST_DIR/linux" ]; then
  echo "  Uploading Linux artifacts..."
  for file in "$DIST_DIR/linux"/*; do
    if [ -f "$file" ]; then
      echo "    Uploading: $(basename "$file")"
      delete_asset "$(basename "$file")"
      gh release upload "$VERSION" "$file" --repo "$REPO"
    fi
  done
fi

# Step 4: Generate and Upload Manifests
echo ""
echo "📄 Step 4: Generating and uploading manifests..."
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

# Start building manifest artifacts with jq
# Initialize with current host CLI artifact
CLI_SIG=""
if [ -f "$DIST_DIR/linggen-cli-${SLUG}.tar.gz.sig.txt" ]; then
  CLI_SIG=$(cat "$DIST_DIR/linggen-cli-${SLUG}.tar.gz.sig.txt")
fi

MANIFEST_JSON=$(jq -n \
  --arg version "${VERSION_NUM}" \
  --arg cli_url "${BASE_URL}/linggen-cli-${SLUG}.tar.gz" \
  --arg cli_key "cli-${SLUG}" \
  --arg cli_sig "$CLI_SIG" \
  '{version: $version, artifacts: {($cli_key): {url: $cli_url, signature: (if $cli_sig != "" then $cli_sig else null end)}}}')

# Add app-macos-tarball if it exists (macOS only)
if [ -f "$DIST_DIR/Linggen.app.tar.gz" ]; then
  UPDATER_SIG=""
  if [ -f "$DIST_DIR/Linggen.app.tar.gz.sig.txt" ]; then
    UPDATER_SIG=$(cat "$DIST_DIR/Linggen.app.tar.gz.sig.txt")
  fi

  MANIFEST_JSON=$(echo "$MANIFEST_JSON" | jq \
    --arg app_url "${BASE_URL}/Linggen.app.tar.gz" \
    --arg app_sig "$UPDATER_SIG" \
    '.artifacts["app-macos-tarball"] = {url: $app_url, signature: (if $app_sig != "" then $app_sig else null end)}')
fi

# Add Linux artifacts if they exist
if [ -d "$DIST_DIR/linux" ]; then
  for arch in x86_64 aarch64; do
    # CLI
    CLI_TAR="linggen-cli-linux-${arch}.tar.gz"
    if [ -f "$DIST_DIR/linux/$CLI_TAR" ]; then
      MANIFEST_JSON=$(echo "$MANIFEST_JSON" | jq \
        --arg url "${BASE_URL}/$CLI_TAR" \
        --arg key "cli-linux-${arch}" \
        '.artifacts[$key] = {url: $url}')
    fi
    # Server
    SRV_TAR="linggen-server-linux-${arch}.tar.gz"
    if [ -f "$DIST_DIR/linux/$SRV_TAR" ]; then
      MANIFEST_JSON=$(echo "$MANIFEST_JSON" | jq \
        --arg url "${BASE_URL}/$SRV_TAR" \
        --arg key "server-linux-${arch}" \
        '.artifacts[$key] = {url: $url}')
    fi
  done
fi

echo "$MANIFEST_JSON" > "$DIST_DIR/manifest.json"

# Generate latest.json (for Tauri)
UPDATER_SIG=""
if [ -f "$DIST_DIR/Linggen.app.tar.gz.sig.txt" ]; then
  UPDATER_SIG=$(cat "$DIST_DIR/Linggen.app.tar.gz.sig.txt")
fi

if [ -n "$UPDATER_SIG" ]; then
  TAURI_PLATFORM="darwin-aarch64"
  if [[ "$SLUG" == *"x86_64"* ]]; then TAURI_PLATFORM="darwin-x86_64"; fi

  jq -n \
    --arg version "$VERSION_NUM" \
    --arg notes "See release notes at https://github.com/${REPO}/releases/tag/${VERSION}" \
    --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg platform "${TAURI_PLATFORM}" \
    --arg signature "$UPDATER_SIG" \
    --arg url "${BASE_URL}/Linggen.app.tar.gz" \
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
fi

delete_asset "manifest.json"
gh release upload "$VERSION" "$DIST_DIR/manifest.json" --repo "$REPO"

if [ -f "$DIST_DIR/latest.json" ]; then
  delete_asset "latest.json"
  gh release upload "$VERSION" "$DIST_DIR/latest.json" --repo "$REPO"
fi

# Step 5: Finalize
if [ "$KEEP_DRAFT" = "true" ]; then
  echo "⚠️  Draft release ${VERSION} created."
else
  echo "🚀 Publishing release..."
  gh release edit "$VERSION" --draft=false --latest --repo "$REPO"
  echo "✅ Release ${VERSION} published!"
  echo "curl -sSL https://linggen.dev/install-cli.sh | bash";
fi
