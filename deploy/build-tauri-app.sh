#!/bin/bash
set -e

# Linggen Tauri Desktop App Builder
# Creates a native desktop app using Tauri

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

APP_NAME="Linggen"
VERSION="0.1.0"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Parse arguments
SKIP_BACKEND=false
CLEAN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-backend)
            SKIP_BACKEND=true
            shift
            ;;
        --clean)
            CLEAN=true
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Usage: $0 [--skip-backend] [--clean]"
            exit 1
            ;;
    esac
done

echo "🖥️  Building Linggen Desktop App with Tauri..."
echo ""

# Clean if requested
if [ "$CLEAN" = true ]; then
    echo -e "${BLUE}🧹 Cleaning previous builds...${NC}"
    rm -rf frontend/src-tauri/target
    rm -rf backend/target
    echo "  Cleaned build directories"
fi

# Detect architecture
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ]; then
    TARGET_TRIPLE="aarch64-apple-darwin"
elif [ "$ARCH" = "x86_64" ]; then
    TARGET_TRIPLE="x86_64-apple-darwin"
else
    echo "Unsupported architecture: $ARCH"
    exit 1
fi

echo -e "${BLUE}Architecture: $ARCH ($TARGET_TRIPLE)${NC}"
echo ""

# Step 1: Build the backend API binary
if [ "$SKIP_BACKEND" = false ]; then
    echo -e "${BLUE}🦀 Building backend (release mode)...${NC}"
    cd backend
    cargo build --release --package api
    cd ..
    echo ""
else
    echo -e "${YELLOW}⏭️  Skipping backend build (--skip-backend flag)${NC}"
    echo ""
fi

# Step 2: Copy backend binary as Tauri sidecar
# Tauri expects sidecars to be named: <sidecar-name>-<target-triple>
SIDECAR_DIR="frontend/src-tauri"
SIDECAR_NAME="linggen-backend-${TARGET_TRIPLE}"
BACKEND_BINARY="backend/target/release/api"

if [ ! -f "$BACKEND_BINARY" ]; then
    echo "Error: Backend binary not found at $BACKEND_BINARY"
    echo "Run without --skip-backend flag to build it first"
    exit 1
fi

echo -e "${BLUE}📦 Setting up sidecar binary...${NC}"
cp "$BACKEND_BINARY" "${SIDECAR_DIR}/${SIDECAR_NAME}"
chmod +x "${SIDECAR_DIR}/${SIDECAR_NAME}"
echo "  ✓ Copied backend binary to ${SIDECAR_DIR}/${SIDECAR_NAME}"
echo ""

# Step 3: Install frontend dependencies if needed
echo -e "${BLUE}📦 Installing frontend dependencies...${NC}"
cd frontend
npm ci
echo ""

# Step 4: Build the Tauri app
echo -e "${BLUE}🏗️  Building Tauri app...${NC}"
echo "  This may take a few minutes..."
# Note: Tauri 2 CLI requires CI=false (not CI=1) if set in the environment
CI=false npm run tauri:build

cd ..

echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║           ✅ Tauri Build Complete!                    ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════╝${NC}"
echo ""

# Copy artifacts to dist/macos
echo -e "${BLUE}📦 Copying artifacts to dist/macos/...${NC}"
mkdir -p dist/macos

# Find and copy DMG
DMG_SRC=$(find frontend/src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -n 1)
APP_SRC="frontend/src-tauri/target/release/bundle/macos/${APP_NAME}.app"

if [ -d "$APP_SRC" ]; then
    rm -rf "dist/macos/${APP_NAME}.app"
    cp -r "$APP_SRC" "dist/macos/"
    echo "  ✓ Copied ${APP_NAME}.app"
fi

if [ -n "$DMG_SRC" ] && [ -f "$DMG_SRC" ]; then
    DMG_NAME=$(basename "$DMG_SRC")
    cp "$DMG_SRC" "dist/macos/$DMG_NAME"
    echo "  ✓ Copied $DMG_NAME"
fi

echo ""
echo "📋 Build Artifacts in dist/macos/:"
echo ""
if [ -d "dist/macos/${APP_NAME}.app" ]; then
    APP_SIZE=$(du -sh "dist/macos/${APP_NAME}.app" | cut -f1)
    echo "  📱 macOS App: dist/macos/${APP_NAME}.app ($APP_SIZE)"
fi

DMG_PATH=$(find dist/macos -name "*.dmg" 2>/dev/null | head -n 1)
if [ -n "$DMG_PATH" ] && [ -f "$DMG_PATH" ]; then
    DMG_SIZE=$(du -sh "$DMG_PATH" | cut -f1)
    echo "  💿 DMG: $DMG_PATH ($DMG_SIZE)"
fi
echo ""

echo "📋 Quick Actions:"
echo ""
echo "  Test the app:"
echo "    open \"dist/macos/${APP_NAME}.app\""
echo ""
echo "  Code sign (optional):"
echo "    codesign --deep --force --sign \"Developer ID Application: Your Name\" \"dist/macos/${APP_NAME}.app\""
echo ""
echo "📋 Development:"
echo ""
echo "  Backend only:"
echo "    cd backend && cargo run --package api"
echo ""
echo "  Tauri dev (uses running backend or starts sidecar):"
echo "    cd frontend && npm run tauri:dev"
echo ""
echo "  Rebuild without backend:"
echo "    ./deploy/build-tauri-app.sh --skip-backend"
echo ""
