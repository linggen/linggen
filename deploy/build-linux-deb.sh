#!/bin/bash
# Build Linggen .deb and .AppImage packages for Linux using Docker

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "🐳 Building Linux packages using Docker..."
echo "   This may take 10-20 minutes on first run (downloading dependencies)"
echo ""

# Build the Docker image and run the build
docker build -f deploy/build-linux.dockerfile -t linggen-linux-builder .

# Create a container and copy the packages out
CONTAINER_ID=$(docker create linggen-linux-builder)
mkdir -p dist/linux

# Copy .deb files
echo "📦 Extracting .deb package..."
docker cp "$CONTAINER_ID:/app/frontend/src-tauri/target/release/bundle/deb/." dist/linux/ 2>/dev/null || true

# Copy .AppImage files
echo "📦 Extracting .AppImage package..."
docker cp "$CONTAINER_ID:/app/frontend/src-tauri/target/release/bundle/appimage/." dist/linux/ 2>/dev/null || true

# Cleanup container
docker rm "$CONTAINER_ID"

echo ""
echo "✅ Build complete! Packages in dist/linux/"
echo ""
echo "Files:"
ls -lh dist/linux/*.deb dist/linux/*.AppImage 2>/dev/null || ls -lh dist/linux/

echo ""
echo "📝 To install on Debian/Ubuntu:"
echo "   sudo dpkg -i dist/linux/linggen_*.deb"
echo ""
echo "📝 To run AppImage:"
echo "   chmod +x dist/linux/linggen_*.AppImage"
echo "   ./dist/linux/linggen_*.AppImage"
