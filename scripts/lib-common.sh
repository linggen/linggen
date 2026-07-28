#!/bin/bash
# Shared library for release and packaging scripts

# Put cargo on PATH. rustup installs it via ~/.cargo/env, which a non-login or
# GUI-spawned shell never sources — a release then dies mid-build with
# "cargo: command not found".
ensure_cargo() {
  command -v cargo >/dev/null 2>&1 && return 0

  # shellcheck disable=SC1091
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
  command -v cargo >/dev/null 2>&1 && return 0

  echo "Error: cargo not found. Install Rust (https://rustup.rs) or put ~/.cargo/bin on PATH." >&2
  return 1
}

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
