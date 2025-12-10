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

# Step 1: Build the backend server binary (now `linggen-server`; CLI is separate)
if [ "$SKIP_BACKEND" = false ]; then
    echo -e "${BLUE}🦀 Building linggen-server (release mode)...${NC}"
    cd backend
    cargo build --release --bin linggen-server
    cd ..
    echo "  ✓ Built linggen-server binary"
    echo ""
else
    echo -e "${YELLOW}⏭️  Skipping backend build (--skip-backend flag)${NC}"
    echo ""
fi

# Step 2: Copy linggen-server binary as Tauri sidecar
# Tauri expects sidecars to be named: <sidecar-name>-<target-triple>
SIDECAR_DIR="frontend/src-tauri"
SIDECAR_NAME="linggen-server-${TARGET_TRIPLE}"
LINGGEN_BINARY="backend/target/release/linggen-server"

if [ ! -f "$LINGGEN_BINARY" ]; then
    echo "Error: linggen-server binary not found at $LINGGEN_BINARY"
    echo "Run without --skip-backend flag to build it first"
    exit 1
fi

echo -e "${BLUE}📦 Setting up sidecar binary...${NC}"
cp "$LINGGEN_BINARY" "${SIDECAR_DIR}/${SIDECAR_NAME}"
chmod +x "${SIDECAR_DIR}/${SIDECAR_NAME}"
echo "  ✓ Copied linggen-server binary to ${SIDECAR_DIR}/${SIDECAR_NAME}"
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
    
    # Create Install CLI.app
    echo -e "${BLUE}🔧 Creating CLI installer app...${NC}"
    bash deploy/create-cli-installer-app.sh "dist/macos"
    
    # Repackage DMG with Install CLI.app included
    echo -e "${BLUE}📦 Repackaging DMG with CLI installer...${NC}"
    DMG_TEMP="dist/macos/dmg_repack"
    rm -rf "$DMG_TEMP"
    mkdir -p "$DMG_TEMP"
    
    # Mount original DMG and copy contents (without pulling in /Applications)
    MOUNT_POINT="/Volumes/${APP_NAME}"
    hdiutil attach "$DMG_SRC" -mountpoint "$MOUNT_POINT" -nobrowse -readonly

    # Copy everything except the Applications alias, which would otherwise
    # expand to the entire /Applications folder on your Mac.
    for item in "$MOUNT_POINT"/*; do
        name="$(basename "$item")"
        if [ "$name" = "Applications" ]; then
            # Recreate the standard /Applications link without expanding it
            ln -s /Applications "$DMG_TEMP/Applications"
        else
            cp -R "$item" "$DMG_TEMP/"
        fi
    done

    hdiutil detach "$MOUNT_POINT"
    
    # Add Install CLI.app to DMG contents
    cp -r "dist/macos/Install CLI.app" "$DMG_TEMP/"
    
    # Create new DMG
    DMG_FINAL="dist/macos/$DMG_NAME"
    rm -f "$DMG_FINAL"
    hdiutil create -volname "${APP_NAME}" \
        -srcfolder "$DMG_TEMP" \
        -ov -format UDZO \
        "$DMG_FINAL"
    
    # Clean up
    rm -rf "$DMG_TEMP"
    rm -rf "dist/macos/Install CLI.app"
    
    echo "  ✓ Created enhanced DMG: $DMG_NAME (with CLI installer)"
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
echo "  Open DMG (includes 'Install CLI' app):"
echo "    open \"$DMG_PATH\""
echo ""
echo "  Install CLI manually (alternative):"
echo "    ./install-cli-from-app.sh"
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
