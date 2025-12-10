#!/bin/bash
set -e

# Creates a clickable "Install CLI.app" for the DMG
# This app will install the linggen CLI to /usr/local/bin

OUTPUT_DIR="${1:-dist/macos}"
APP_PATH="${OUTPUT_DIR}/Install CLI.app"

echo "Creating Install CLI.app..."

# Create app bundle structure
mkdir -p "${APP_PATH}/Contents/MacOS"
mkdir -p "${APP_PATH}/Contents/Resources"

# Create Info.plist
cat > "${APP_PATH}/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>install-cli</string>
    <key>CFBundleIdentifier</key>
    <string>dev.linggen.cli-installer</string>
    <key>CFBundleName</key>
    <string>Install CLI</string>
    <key>CFBundleDisplayName</key>
    <string>Install Linggen CLI</string>
    <key>CFBundleVersion</key>
    <string>1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSUIElement</key>
    <false/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

# Create the installer script
cat > "${APP_PATH}/Contents/MacOS/install-cli" << 'INSTALLER_SCRIPT'
#!/bin/bash

# macOS CLI Installer for Linggen
# This script is run when users double-click "Install CLI.app" from the DMG

APP_PATH="/Applications/Linggen.app"
CLI_NAME="linggen"
INSTALL_DIR="/usr/local/bin"

# Show GUI dialogs using osascript
show_error() {
    osascript -e "display dialog \"$1\" buttons {\"OK\"} default button \"OK\" with icon stop with title \"Linggen CLI Installer\""
}

show_success() {
    osascript -e "display dialog \"$1\" buttons {\"OK\"} default button \"OK\" with icon note with title \"Linggen CLI Installer\""
}

show_prompt() {
    osascript -e "display dialog \"$1\" buttons {\"Cancel\", \"Install\"} default button \"Install\" with icon note with title \"Linggen CLI Installer\""
}

# Check if app is installed
if [ ! -d "$APP_PATH" ]; then
    show_error "Linggen.app not found at $APP_PATH\n\nPlease install Linggen.app first by dragging it to the Applications folder."
    exit 1
fi

# Find the CLI binary in the app
CLI_BINARY="${APP_PATH}/Contents/MacOS/linggen"

if [ ! -f "$CLI_BINARY" ]; then
    show_error "CLI binary not found in Linggen.app\n\nPath checked: $CLI_BINARY"
    exit 1
fi

# Check if already installed
SYMLINK_PATH="${INSTALL_DIR}/${CLI_NAME}"
if [ -L "$SYMLINK_PATH" ] || [ -f "$SYMLINK_PATH" ]; then
    EXISTING_TARGET=$(readlink "$SYMLINK_PATH" 2>/dev/null || echo "$SYMLINK_PATH")
    
    # If it's already pointing to the right place, we're done
    if [ "$EXISTING_TARGET" = "$CLI_BINARY" ]; then
        show_success "✅ Linggen CLI is already installed!\n\nYou can use it from Terminal:\n  linggen --help\n  linggen index /path/to/project"
        exit 0
    fi
    
    # Ask to replace existing installation
    if ! show_prompt "⚠️ A linggen command already exists at:\n$SYMLINK_PATH\n\nDo you want to replace it with the version from Linggen.app?"; then
        exit 0
    fi
fi

# Prompt for installation
if ! show_prompt "This will install the linggen CLI to:\n$SYMLINK_PATH\n\nYou may be prompted for your password.\n\nContinue?"; then
    exit 0
fi

# Create install directory if needed
if [ ! -d "$INSTALL_DIR" ]; then
    osascript -e "do shell script \"mkdir -p '$INSTALL_DIR'\" with administrator privileges" || {
        show_error "Failed to create $INSTALL_DIR"
        exit 1
    }
fi

# Remove old symlink if exists
if [ -L "$SYMLINK_PATH" ] || [ -f "$SYMLINK_PATH" ]; then
    osascript -e "do shell script \"rm -f '$SYMLINK_PATH'\" with administrator privileges" || {
        show_error "Failed to remove existing installation"
        exit 1
    }
fi

# Create symlink
osascript -e "do shell script \"ln -s '$CLI_BINARY' '$SYMLINK_PATH'\" with administrator privileges" || {
    show_error "Failed to create symlink"
    exit 1
}

# Verify installation
if command -v linggen >/dev/null 2>&1; then
    show_success "✅ Linggen CLI installed successfully!\n\nYou can now use it from Terminal:\n  linggen --help\n  linggen serve\n  linggen index /path/to/project\n\nInstalled to: $SYMLINK_PATH"
else
    show_success "✅ Linggen CLI installed to:\n$SYMLINK_PATH\n\nNote: You may need to restart your Terminal or add $INSTALL_DIR to your PATH."
fi
INSTALLER_SCRIPT

chmod +x "${APP_PATH}/Contents/MacOS/install-cli"

echo "✅ Created: ${APP_PATH}"
