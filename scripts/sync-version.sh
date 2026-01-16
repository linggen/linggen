#!/bin/bash
set -euo pipefail

# Sync version to all project files
# Usage: ./scripts/sync-version.sh <version>
#        Version should be without 'v' prefix (e.g., "0.2.2")

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 0.2.2" >&2
  exit 1
fi

# Remove 'v' prefix if present
VERSION="${VERSION#v}"

echo "🔄 Syncing version $VERSION to all project files..."

# Update linggen-cli/Cargo.toml
if [ -f "$ROOT_DIR/linggen-cli/Cargo.toml" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/linggen-cli/Cargo.toml"
  else
    # Linux
    sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/linggen-cli/Cargo.toml"
  fi
  echo "  ✅ Updated linggen-cli/Cargo.toml"
  
  # Update Cargo.lock for linggen-cli
  (cd "$ROOT_DIR/linggen-cli" && cargo fetch 2>/dev/null || true)
  echo "  ✅ Updated linggen-cli/Cargo.lock"
fi

# Update frontend/src-tauri/Cargo.toml
if [ -f "$ROOT_DIR/frontend/src-tauri/Cargo.toml" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/frontend/src-tauri/Cargo.toml"
  else
    sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/frontend/src-tauri/Cargo.toml"
  fi
  echo "  ✅ Updated frontend/src-tauri/Cargo.toml"
fi

# Update frontend/package.json
if [ -f "$ROOT_DIR/frontend/package.json" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/frontend/package.json"
  else
    sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/frontend/package.json"
  fi
  echo "  ✅ Updated frontend/package.json"
fi

# Update frontend/src-tauri/tauri.conf.json
if [ -f "$ROOT_DIR/frontend/src-tauri/tauri.conf.json" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/frontend/src-tauri/tauri.conf.json"
  else
    sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/frontend/src-tauri/tauri.conf.json"
  fi
  echo "  ✅ Updated frontend/src-tauri/tauri.conf.json"
fi

# Update backend/Cargo.toml (workspace version)
if [ -f "$ROOT_DIR/backend/Cargo.toml" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/backend/Cargo.toml"
  else
    sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/backend/Cargo.toml"
  fi
  echo "  ✅ Updated backend/Cargo.toml"
  
  # Update workspace Cargo.lock
  (cd "$ROOT_DIR/backend" && cargo fetch 2>/dev/null || true)
  echo "  ✅ Updated backend/Cargo.lock"
fi

echo "✅ Version sync complete!"
