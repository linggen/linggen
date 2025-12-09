#!/bin/bash
set -e

# Install Linggen CLI from the installed macOS app
# This script creates a symlink from the bundled CLI to /usr/local/bin

APP_PATH="/Applications/Linggen.app"
CLI_NAME="linggen"
INSTALL_DIR="/usr/local/bin"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}🔧 Linggen CLI Installer${NC}"
echo ""

# Check if app is installed
if [ ! -d "$APP_PATH" ]; then
    echo -e "${RED}❌ Linggen.app not found at $APP_PATH${NC}"
    echo "Please install Linggen.app first by:"
    echo "  1. Opening the Linggen DMG"
    echo "  2. Dragging Linggen.app to Applications"
    exit 1
fi

# Detect architecture
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ]; then
    TARGET_TRIPLE="aarch64-apple-darwin"
elif [ "$ARCH" = "x86_64" ]; then
    TARGET_TRIPLE="x86_64-apple-darwin"
else
    echo -e "${RED}Unsupported architecture: $ARCH${NC}"
    exit 1
fi

CLI_BINARY="${APP_PATH}/Contents/MacOS/linggen-cli-${TARGET_TRIPLE}"

# Check if CLI binary exists in app
if [ ! -f "$CLI_BINARY" ]; then
    echo -e "${RED}❌ CLI binary not found at $CLI_BINARY${NC}"
    echo "Your Linggen.app may not include the CLI."
    echo "Please download a version that includes the CLI or build from source."
    exit 1
fi

echo "Found CLI binary: $CLI_BINARY"
echo ""

# Check if install directory exists, create if not
if [ ! -d "$INSTALL_DIR" ]; then
    echo -e "${YELLOW}Creating $INSTALL_DIR...${NC}"
    sudo mkdir -p "$INSTALL_DIR"
fi

# Install (create symlink)
SYMLINK_PATH="${INSTALL_DIR}/${CLI_NAME}"

if [ -L "$SYMLINK_PATH" ] || [ -f "$SYMLINK_PATH" ]; then
    echo -e "${YELLOW}⚠️  $SYMLINK_PATH already exists${NC}"
    read -p "Do you want to replace it? (y/n) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        echo "Removing existing $SYMLINK_PATH..."
        sudo rm -f "$SYMLINK_PATH"
    else
        echo "Cancelled."
        exit 0
    fi
fi

echo -e "${BLUE}Creating symlink...${NC}"
sudo ln -s "$CLI_BINARY" "$SYMLINK_PATH"

echo ""
echo -e "${GREEN}✅ Linggen CLI installed successfully!${NC}"
echo ""
echo "You can now use the CLI from anywhere:"
echo "  ${CLI_NAME} --help"
echo "  ${CLI_NAME} start"
echo "  ${CLI_NAME} index /path/to/project"
echo ""

# Verify installation
if command -v linggen >/dev/null 2>&1; then
    echo -e "${GREEN}✓ Verified: 'linggen' is now available on your PATH${NC}"
else
    echo -e "${YELLOW}⚠️  'linggen' not found on PATH${NC}"
    echo "You may need to restart your terminal or add $INSTALL_DIR to your PATH"
fi
