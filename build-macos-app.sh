#!/bin/bash
set -e

# Linggen macOS App Builder
# Creates a .app bundle and .dmg for distribution

APP_NAME="Linggen"
VERSION="0.1.0"
BUNDLE_ID="dev.linggen.app"
ARCH=$(uname -m)  # arm64 or x86_64

echo "🍎 Building Linggen.app for macOS ($ARCH)..."

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

# Clean previous builds
rm -rf dist/macos
mkdir -p dist/macos

# Build frontend
echo -e "${BLUE}📦 Building frontend...${NC}"
cd frontend
npm ci
npm run build
cd ..

# Build backend (release)
echo -e "${BLUE}🦀 Building backend (release mode)...${NC}"
cd backend
cargo build --release --package api
cd ..

# Create .app bundle structure
echo -e "${BLUE}📁 Creating app bundle...${NC}"
APP_DIR="dist/macos/${APP_NAME}.app"
mkdir -p "${APP_DIR}/Contents/MacOS"
mkdir -p "${APP_DIR}/Contents/Resources"
mkdir -p "${APP_DIR}/Contents/Resources/frontend"
mkdir -p "${APP_DIR}/Contents/Resources/data"

# Copy binary
cp backend/target/release/api "${APP_DIR}/Contents/MacOS/linggen"

# Copy frontend
cp -r frontend/dist/* "${APP_DIR}/Contents/Resources/frontend/"

# Create Info.plist
cat > "${APP_DIR}/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>linggen-launcher</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSUIElement</key>
    <false/>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2024 Linggen. All rights reserved.</string>
</dict>
</plist>
EOF

# Create launcher script (opens browser + runs server)
cat > "${APP_DIR}/Contents/MacOS/linggen-launcher" << 'EOF'
#!/bin/bash
DIR="$(cd "$(dirname "$0")" && pwd)"
RESOURCES_DIR="$(dirname "$DIR")/Resources"

# Change to Resources directory so relative paths work
cd "$RESOURCES_DIR"

# Open browser after a short delay
(sleep 2 && open "http://localhost:7000") &

# Run the server
exec "$DIR/linggen"
EOF
chmod +x "${APP_DIR}/Contents/MacOS/linggen-launcher"

# Create a simple icon (you should replace with real icon)
# For now, create placeholder
if [ -f "assets/icon.icns" ]; then
    cp assets/icon.icns "${APP_DIR}/Contents/Resources/AppIcon.icns"
else
    echo -e "${BLUE}⚠️  No icon found at assets/icon.icns - using placeholder${NC}"
fi

echo -e "${GREEN}✅ App bundle created: ${APP_DIR}${NC}"

# Create DMG
echo -e "${BLUE}💿 Creating DMG...${NC}"
DMG_NAME="Linggen-${VERSION}-${ARCH}.dmg"
DMG_PATH="dist/macos/${DMG_NAME}"

# Create temporary DMG directory
DMG_TEMP="dist/macos/dmg_temp"
rm -rf "$DMG_TEMP"
mkdir -p "$DMG_TEMP"
cp -r "${APP_DIR}" "$DMG_TEMP/"

# Create symlink to Applications
ln -s /Applications "$DMG_TEMP/Applications"

# Create DMG
hdiutil create -volname "${APP_NAME}" \
    -srcfolder "$DMG_TEMP" \
    -ov -format UDZO \
    "$DMG_PATH"

# Clean up
rm -rf "$DMG_TEMP"

echo -e "${GREEN}✅ DMG created: ${DMG_PATH}${NC}"
echo ""
echo "📋 Next steps:"
echo "   1. Test: open dist/macos/${APP_NAME}.app"
echo "   2. (Optional) Code sign: codesign --deep --force --sign 'Developer ID' ${APP_DIR}"
echo "   3. (Optional) Notarize for Gatekeeper"
echo "   4. Upload ${DMG_NAME} to linggen.dev"

