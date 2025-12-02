#!/bin/bash
set -e

# Linggen macOS App Builder
# Creates a .app bundle and .dmg for distribution

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

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

# App icon handling
# Prefer a pre-generated assets/icon.icns, but if it's missing and we have a PNG,
# auto-generate icon.icns from the PNG using sips + iconutil (macOS tools).
if [ -f "assets/icon.icns" ]; then
    cp "assets/icon.icns" "${APP_DIR}/Contents/Resources/AppIcon.icns"
else
    # Try to find a PNG to use as the source icon.
    ICON_SOURCE_PNG=""

    # 1) Explicit app icon in assets/ if present
    if [ -f "assets/icon.png" ]; then
        ICON_SOURCE_PNG="assets/icon.png"
    # 2) Prefer the Linggen site logo if available
    elif [ -f "linggensite/src/assets/logo.png" ]; then
        ICON_SOURCE_PNG="linggensite/src/assets/logo.png"
    else
        # 3) Fall back to the first PNG in assets (e.g. a Figma-exported asset)
        PNG_CANDIDATES=(assets/*.png)
        if [ -f "${PNG_CANDIDATES[0]}" ]; then
            ICON_SOURCE_PNG="${PNG_CANDIDATES[0]}"
        fi
    fi

    if [ -n "$ICON_SOURCE_PNG" ]; then
        echo -e "${BLUE}🎨 Generating assets/icon.icns from ${ICON_SOURCE_PNG}...${NC}"

        if command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
            ICON_TMP_DIR="$(mktemp -d /tmp/linggen_icon.XXXXXX)"
            ICONSET_DIR="${ICON_TMP_DIR}/icon.iconset"
            mkdir -p "$ICONSET_DIR"

            # Force output format to PNG so iconutil is happy even if the
            # source file is actually a JPEG with a .png extension.
            sips -s format png -z 16 16     "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_16x16.png"
            sips -s format png -z 32 32     "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_16x16@2x.png"
            sips -s format png -z 32 32     "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_32x32.png"
            sips -s format png -z 64 64     "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_32x32@2x.png"
            sips -s format png -z 128 128   "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_128x128.png"
            sips -s format png -z 256 256   "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_128x128@2x.png"
            sips -s format png -z 256 256   "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_256x256.png"
            sips -s format png -z 512 512   "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_256x256@2x.png"
            sips -s format png -z 512 512   "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_512x512.png"
            sips -s format png -z 1024 1024 "$ICON_SOURCE_PNG" --out "$ICONSET_DIR/icon_512x512@2x.png"

            iconutil -c icns "$ICONSET_DIR" -o "assets/icon.icns"
            rm -rf "$ICON_TMP_DIR"

            if [ -f "assets/icon.icns" ]; then
                cp "assets/icon.icns" "${APP_DIR}/Contents/Resources/AppIcon.icns"
            else
                echo -e "${BLUE}⚠️  Failed to generate assets/icon.icns - app will use default icon${NC}"
            fi
        else
            echo -e "${BLUE}⚠️  'sips' and/or 'iconutil' not found - cannot generate icon.icns from PNG${NC}"
        fi
    else
        echo -e "${BLUE}⚠️  No PNG found in assets/ to generate an icon from - app will use default icon${NC}"
    fi
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

