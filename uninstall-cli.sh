#!/bin/bash
set -e

# Uninstall Linggen CLI symlink

CLI_NAME="linggen"
INSTALL_DIR="/usr/local/bin"
SYMLINK_PATH="${INSTALL_DIR}/${CLI_NAME}"

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "🗑️  Uninstalling Linggen CLI..."
echo ""

if [ ! -L "$SYMLINK_PATH" ] && [ ! -f "$SYMLINK_PATH" ]; then
    echo -e "${YELLOW}No installation found at $SYMLINK_PATH${NC}"
    exit 0
fi

echo "Removing $SYMLINK_PATH..."
sudo rm -f "$SYMLINK_PATH"

echo ""
echo -e "${GREEN}✅ Linggen CLI uninstalled successfully${NC}"
