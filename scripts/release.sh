#!/bin/bash
set -euo pipefail

# Release orchestrator script for Linggen Agent
# Usage: ./scripts/release.sh <version> [--draft] [--platform mac|linux]
#        ./scripts/release.sh <version> --linux-ci        # dispatch the cloud Linux build
#        ./scripts/release.sh <version> --manifest-only   # regenerate manifest.json only
#
# Default platform is the current host (no cross-build):
#   - macOS host  → mac
#   - Linux host  → linux (multi-arch: x86_64 + aarch64)
#
# Cutting mac-first, then Linux, is a three-step flow — the manifest is written
# from the release's live asset list, so it can only list Linux once those
# assets exist:
#   ./scripts/release.sh 1.6.1 --draft      # mac assets + a first manifest
#   ./scripts/release.sh 1.6.1 --linux-ci   # cloud build uploads the linux ones
#   ./scripts/release.sh 1.6.1 --manifest-only   # rewrite the manifest with all three

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

REPO="linggen/linggen"
VERSION=""
KEEP_DRAFT=false
PLATFORM=""
MODE="full"
PASS_ARGS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --draft)
      KEEP_DRAFT=true
      shift ;;
    --manifest-only)
      MODE="manifest"
      shift ;;
    --linux-ci)
      MODE="linux-ci"
      shift ;;
    --platform)
      PLATFORM="${2:-}"
      PASS_ARGS+=("--platform" "$PLATFORM")
      shift 2 ;;
    --platform=*)
      PLATFORM="${1#--platform=}"
      PASS_ARGS+=("$1")
      shift ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      fi
      shift ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--draft] [--platform mac|linux]" >&2
  echo "       $0 <version> --linux-ci" >&2
  echo "       $0 <version> --manifest-only" >&2
  exit 1
fi

OS_LOWER="$(uname -s | tr '[:upper:]' '[:lower:]')"
HOST_PLATFORM="$([ "$OS_LOWER" = "darwin" ] && echo mac || echo linux)"
PLATFORM="${PLATFORM:-$HOST_PLATFORM}"

case "$PLATFORM" in
  mac|linux) ;;
  *)
    echo "Error: --platform must be 'mac' or 'linux' (got '$PLATFORM')" >&2
    exit 1 ;;
esac

VERSION_NUM="${VERSION#v}"
DIST_DIR="$ROOT_DIR/dist"

delete_asset() {
  local name="$1"
  gh release delete-asset "$VERSION" "$name" --repo "$REPO" --yes 2>/dev/null || true
}

# The git tag doesn't exist until the draft is published, and it is created on
# the default branch at that moment — so the version stamp has to be pushed
# first, or the tag lands on a commit that still carries the old version.
commit_version_stamp() {
  local stamped=(Cargo.toml Cargo.lock ui/package.json)
  cd "$ROOT_DIR"

  for file in "${stamped[@]}"; do
    git ls-files --error-unmatch "$file" >/dev/null 2>&1 && git add -- "$file"
  done

  if git diff --cached --quiet; then
    echo "✅ Version stamp already committed"
    return 0
  fi

  git commit -m "chore: release ${VERSION_NUM}"
  git push origin "HEAD:$(git symbolic-ref --short HEAD)"
  echo "✅ Committed and pushed the version stamp"
}

ensure_release() {
  if gh release view "$VERSION" --repo "$REPO" &>/dev/null; then
    echo "✅ Release ${VERSION} already exists"
    return 0
  fi
  gh release create "$VERSION" \
    --repo "$REPO" \
    --title "Linggen ${VERSION}" \
    --notes "Release ${VERSION}" \
    --draft
  echo "✅ Created draft release ${VERSION}"
}

upload_artifacts() {
  local slug
  slug=$(detect_platform)

  local tarball="$DIST_DIR/ling-${slug}.tar.gz"
  local has_mac_tarball=false
  local has_linux_dir=false
  [ "$PLATFORM" = "mac" ] && [ -f "$tarball" ] && has_mac_tarball=true
  [ -d "$DIST_DIR/linux" ] && has_linux_dir=true

  if [ "$has_mac_tarball" = "false" ] && [ "$has_linux_dir" = "false" ]; then
    echo "Error: no artifacts to upload — did the build step produce anything?" >&2
    echo "Looked for: $tarball and $DIST_DIR/linux/" >&2
    exit 1
  fi

  # ling binary tarball (mac platform only — linux artifacts live under dist/linux/)
  if [ "$has_mac_tarball" = "true" ]; then
    echo "  Uploading: $(basename "$tarball")"
    delete_asset "$(basename "$tarball")"
    gh release upload "$VERSION" "$tarball" --repo "$REPO"
  fi

  # Linux Artifacts (multi-arch from Docker)
  if [ "$has_linux_dir" = "true" ]; then
    echo "  Uploading Linux artifacts..."
    for file in "$DIST_DIR/linux"/*; do
      if [ -f "$file" ]; then
        echo "    Uploading: $(basename "$file")"
        delete_asset "$(basename "$file")"
        gh release upload "$VERSION" "$file" --repo "$REPO"
      fi
    done
  fi
}

# Build the assets array from the release's actual asset list, not just local
# dist/. This makes split-host workflows additive: a mac run and a later linux
# run keep both sets of entries instead of the second overwriting the first.
# Rerun with --manifest-only once late assets (the cloud Linux build) land.
generate_manifest() {
  local base_url="https://github.com/${REPO}/releases/download/${VERSION}"
  mkdir -p "$DIST_DIR"

  local assets
  assets=$(gh release view "$VERSION" --repo "$REPO" --json assets \
    | jq --arg base "$base_url" \
        '[.assets[]
           | select(.name | test("^ling-.*\\.tar\\.gz$"))
           | {name: (.name | sub("\\.tar\\.gz$"; "")),
              url: ($base + "/" + .name)}]')

  jq -n \
    --arg version "${VERSION_NUM}" \
    --argjson assets "$assets" \
    '{version: $version, assets: $assets}' > "$DIST_DIR/manifest.json"

  echo "  Manifest lists: $(jq -r '[.assets[].name] | join(", ")' "$DIST_DIR/manifest.json")"

  delete_asset "manifest.json"
  gh release upload "$VERSION" "$DIST_DIR/manifest.json" --repo "$REPO"
}

# actions/checkout cannot resolve a short sha ("The process '/usr/bin/git'
# failed with exit code 1"), and without release_tag the run produces
# artifacts and uploads nothing. Pass both, always.
dispatch_linux_ci() {
  local sha
  sha=$(git -C "$ROOT_DIR" rev-parse HEAD)
  echo "🐧 Dispatching build-linux.yml at ${sha} → ${VERSION}"
  gh workflow run build-linux.yml \
    --repo "$REPO" \
    -f ref="$sha" \
    -f release_tag="$VERSION"
  echo "✅ Dispatched. Watch: gh run list --repo ${REPO} --workflow build-linux.yml"
  echo "   When it lands: $0 ${VERSION} --manifest-only"
}

finalize() {
  if [ "$KEEP_DRAFT" = "true" ]; then
    echo "⚠️  Draft release ${VERSION} created."
    return 0
  fi
  echo "🚀 Publishing release..."
  gh release edit "$VERSION" --draft=false --latest --repo "$REPO"
  echo "✅ Release ${VERSION} published!"
  echo "curl -fsSL https://linggen.dev/install.sh | bash"
}

case "$MODE" in
  manifest)
    echo "📄 Regenerating manifest for ${VERSION}..."
    generate_manifest
    exit 0 ;;
  linux-ci)
    dispatch_linux_ci
    exit 0 ;;
esac

# Step 1: Build everything
echo "📦 Step 1: Building all artifacts..."
"$ROOT_DIR/scripts/build.sh" "$VERSION" ${PASS_ARGS[@]+"${PASS_ARGS[@]}"}

# Step 2: Commit the version stamp the build just wrote
echo ""
echo "🔖 Step 2: Committing version stamp..."
commit_version_stamp

# Step 3: Create GitHub Release
echo ""
echo "🚀 Step 3: Creating GitHub Release..."
ensure_release

# Step 4: Upload Artifacts
echo ""
echo "📤 Step 4: Uploading artifacts..."
upload_artifacts

# Step 5: Generate and Upload Manifest
echo ""
echo "📄 Step 5: Generating and uploading manifest..."
generate_manifest

# Step 6: Finalize
finalize
