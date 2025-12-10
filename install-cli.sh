#!/usr/bin/env bash
set -euo pipefail

# Bash-only guard (avoid fish/zsh sourcing issues)
if [ -n "${FISH_VERSION:-}" ]; then
  echo "This script is bash-only. Run: bash install-cli.sh"
  exit 1
fi

INSTALL_DIR_DEFAULT="/usr/local/bin"
FALLBACK_DIR="$HOME/.local/bin"

LOCAL_PATH=""
VERSION_ARG=""

usage() {
  cat <<EOF
Usage: $(basename "$0") [--version <ver>] [--local-path <file://...tar.gz>]

Options:
  --version <ver>    Install a specific version (expects asset naming with v<ver>). If omitted, installs latest.
  --local-path <url> Install from a local file URL or local path (file://...). Skips network fetch.

If neither is provided, installs from the latest published tarball for this platform.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION_ARG="$2"; shift 2 ;;
    --local-path)
      LOCAL_PATH="$2"; shift 2 ;;
    -h|--help)
      usage; exit 0 ;;
    *)
      echo "Unknown option: $1" >&2
      usage; exit 1 ;;
  esac
done

detect_slug() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "$os" in
    darwin)
      case "$arch" in
        arm64|aarch64) echo "macos-aarch64" ;;
        x86_64|amd64)  echo "macos-x86_64" ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
      esac
      ;;
    linux)
      case "$arch" in
        x86_64|amd64) echo "linux-x86_64" ;;
        arm64|aarch64) echo "linux-arm64" ;;
        *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
      esac
      ;;
    *) echo "Unsupported OS: $os" >&2; exit 1 ;;
  esac
}

ensure_dir() {
  local dir="$1"
  if [ ! -d "$dir" ]; then
    if mkdir -p "$dir" 2>/dev/null; then
      return 0
    elif command -v sudo >/dev/null 2>&1; then
      sudo mkdir -p "$dir"
    else
      return 1
    fi
  fi
  if [ -w "$dir" ]; then
    return 0
  elif command -v sudo >/dev/null 2>&1; then
    return 0  # will rely on sudo for copy
  else
    return 1
  fi
}

install_binary() {
  local tarball="$1" dest_dir="$2"
  local tmpdir binpath
  tmpdir="$(mktemp -d)"
  tar -xzf "$tarball" -C "$tmpdir"
  binpath="$tmpdir/linggen"
  if [ ! -f "$binpath" ]; then
    echo "linggen binary not found in tarball" >&2
    exit 1
  fi

  if [ -w "$dest_dir" ]; then
    cp "$binpath" "$dest_dir/"
  else
    sudo cp "$binpath" "$dest_dir/"
  fi
  chmod +x "$dest_dir/linggen"
}

main() {
  local slug url tarball dest="$INSTALL_DIR_DEFAULT"
  local version="${VERSION_ARG:-latest}"

  slug="$(detect_slug)"

  if [ -n "$LOCAL_PATH" ]; then
    # Accept file:// or plain path
    if [[ "$LOCAL_PATH" == file://* ]]; then
      url="${LOCAL_PATH#file://}"
      url="file://$url"
    else
      url="file://$LOCAL_PATH"
    fi
  elif [ "$version" = "latest" ]; then
    url="https://github.com/linggen/linggen-releases/releases/latest/download/linggen-cli-${slug}.tar.gz"
  else
    # Use versioned release tag but base filename (single asset naming)
    url="https://github.com/linggen/linggen-releases/releases/download/v${version}/linggen-cli-${slug}.tar.gz"
  fi

  echo "➡️  Downloading $url"
  tarball="$(mktemp)"
  if [[ "$url" == file://* ]]; then
    cp "${url#file://}" "$tarball"
  else
    curl -fsSL "$url" -o "$tarball"
  fi

  if ! ensure_dir "$dest"; then
    echo "Using fallback install dir: $FALLBACK_DIR"
    dest="$FALLBACK_DIR"
    mkdir -p "$dest"
  fi

  install_binary "$tarball" "$dest"

  echo "✅ Installed linggen to $dest"
  echo "Version:"
  "$dest/linggen" --version || true
  if [[ ":$PATH:" != *":$dest:"* ]]; then
    echo "ℹ️  Add to PATH if needed:"
    echo "    export PATH=\"$dest:\$PATH\""
  fi
}

main "$@"
