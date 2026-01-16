#!/bin/bash
# Shared library for release and packaging scripts

# Detect platform and set SLUG
detect_platform() {
  local OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
  local ARCH="$(uname -m)"
  local SLUG=""

  case "$OS" in
    darwin)
      case "$ARCH" in
        arm64|aarch64) SLUG="macos-aarch64" ;;
        x86_64|amd64)  SLUG="macos-x86_64" ;;
        *) echo "Unsupported macOS arch: $ARCH" >&2; return 1 ;;
      esac
      ;;
    linux)
      case "$ARCH" in
        x86_64|amd64) SLUG="linux-x86_64" ;;
        arm64|aarch64) SLUG="linux-aarch64" ;;
        *) echo "Unsupported Linux arch: $ARCH" >&2; return 1 ;;
      esac
      ;;
    *)
      echo "Unsupported OS: $OS" >&2; return 1
      ;;
  esac
  echo "$SLUG"
}

# Helper to sign a file and return the signature (base64 encoded full signature)
# Usage: sign_file <file_path> <root_dir>
sign_file() {
  local file_path="$1"
  local root_dir="$2"
  local sig_file="${file_path}.sig"
  
  # Read password from config file or environment variable
  local password=""
  local config_file="$root_dir/.tauri-signing.conf"
  if [ -f "$config_file" ]; then
    # Read password from config file (strip quotes and comments)
    password=$(grep -E "^TAURI_PRIVATE_KEY_PASSWORD=" "$config_file" | grep -v "^#" | cut -d'=' -f2- | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' | tr -d '"' | tr -d "'")
  fi
  
  # Fallback to environment variable, then empty string
  password="${password:-${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-${TAURI_PRIVATE_KEY_PASSWORD:-}}}"
  
  # Read key content and sign using -k (string) option
  local KEY_CONTENT=""
  if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    KEY_CONTENT="$TAURI_SIGNING_PRIVATE_KEY"
  elif [ -n "${TAURI_PRIVATE_KEY:-}" ]; then
    KEY_CONTENT="$TAURI_PRIVATE_KEY"
  elif [ -f "$HOME/.tauri/linggen.key" ]; then
    KEY_CONTENT="$(cat "$HOME/.tauri/linggen.key")"
  else
    echo "⚠️  No signing key found. Set TAURI_PRIVATE_KEY or create ~/.tauri/linggen.key" >&2
    return 1
  fi
  
  # Sign using key content as string
  if npx tauri signer sign -k "$KEY_CONTENT" -p "$password" "$file_path" >/dev/null 2>&1; then
    if [ -f "$sig_file" ]; then
      local first_line
      first_line="$(head -n 1 "$sig_file" 2>/dev/null || true)"
      if echo "$first_line" | grep -q '^untrusted comment:'; then
        base64 -i "$sig_file" | tr -d '\n'
      else
        tr -d '\n' < "$sig_file"
      fi
    else
      return 1
    fi
  else
    echo "⚠️  Signing failed for $file_path" >&2
    return 1
  fi
}
