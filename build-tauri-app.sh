#!/bin/bash
set -e

# Linggen Tauri Desktop App Builder
# Creates a native desktop app using Tauri

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

# Find the actual DMG file
DMG_PATH=$(find frontend/src-tauri/target/release/bundle/dmg -name "*.dmg" 2>/dev/null | head -n 1)
APP_PATH="frontend/src-tauri/target/release/bundle/macos/${APP_NAME}.app"

echo "📋 Build Artifacts:"
echo ""
if [ -d "$APP_PATH" ]; then
    APP_SIZE=$(du -sh "$APP_PATH" | cut -f1)
    echo "  📱 macOS App:"
    echo "     $APP_PATH"
    echo "     Size: $APP_SIZE"
else
    echo "  ⚠️  macOS App not found (check for errors above)"
fi
echo ""

if [ -n "$DMG_PATH" ] && [ -f "$DMG_PATH" ]; then
    DMG_SIZE=$(du -sh "$DMG_PATH" | cut -f1)
    echo "  💿 DMG Installer:"
    echo "     $DMG_PATH"
    echo "     Size: $DMG_SIZE"
else
    echo "  ⚠️  DMG not found (check for errors above)"
fi
echo ""

echo "📋 Quick Actions:"
echo ""
echo "  Test the app:"
echo "    open \"$APP_PATH\""
echo ""
echo "  Code sign (optional):"
echo "    codesign --deep --force --sign \"Developer ID Application: Your Name\" \"$APP_PATH\""
echo ""
echo "  Create a signed DMG:"
echo "    codesign --deep --force --sign \"Developer ID Application: Your Name\" \"$APP_PATH\""
echo "    hdiutil create -volname Linggen -srcfolder \"$APP_PATH\" -ov -format UDZO Linggen-signed.dmg"
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
echo "    ./build-tauri-app.sh --skip-backend"
echo ""
