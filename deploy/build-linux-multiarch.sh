#!/bin/bash
# Build Linggen .deb and .AppImage packages for Linux (x86_64 and arm64) using Docker Buildx

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

echo "🐳 Building multi-arch Linux packages using Docker Buildx..."
echo "   Target architectures: amd64 (x86_64), arm64 (aarch64)"
echo "   This will take a while as it builds for both architectures."
echo ""

# Ensure buildx is available and set up
if ! docker buildx version > /dev/null 2>&1; then
    echo "❌ Error: Docker Buildx is not installed or enabled."
    exit 1
fi

# Create and use a new builder if needed (supports multi-arch)
BUILDER_NAME="linggen-builder"
if ! docker buildx inspect "$BUILDER_NAME" > /dev/null 2>&1; then
    echo "🔧 Creating new Buildx builder: $BUILDER_NAME"
    docker buildx create --name "$BUILDER_NAME" --use
else
    docker buildx use "$BUILDER_NAME"
fi

# Ensure output directory exists
mkdir -p dist/linux

# Run the build
echo "🚀 Starting build process..."
docker buildx build \
    --platform linux/amd64,linux/arm64 \
    --target artifacts \
    --output type=local,dest=./dist/linux \
    -f deploy/Dockerfile.linux.multiarch \
    .

echo ""
echo "✅ Multi-arch build complete! Packages are in dist/linux/"
echo ""
echo "Files found:"
ls -lh dist/linux/*.deb dist/linux/*.AppImage 2>/dev/null || ls -lh dist/linux/

echo ""
echo "📝 To install on Debian/Ubuntu (amd64):"
echo "   sudo dpkg -i dist/linux/linggen_*_amd64.deb"
echo ""
echo "📝 To install on Debian/Ubuntu (arm64):"
echo "   sudo dpkg -i dist/linux/linggen_*_arm64.deb"

