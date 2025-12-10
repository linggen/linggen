# Deploy Guide

This guide covers building, testing, and releasing Linggen artifacts.

## Overview

This repo ships separate artifacts:

- `linggen` (standalone CLI)
- `linggen-server` (backend/server; Tauri sidecar)
- `Linggen.app` (macOS desktop)

## Prerequisites

1. **Rust toolchain** installed:

   ```bash
   rustc --version
   cargo --version
   ```

2. **Node.js** installed (for Tauri app):

   ```bash
   node --version
   npm --version
   ```

3. **GitHub CLI (`gh`)** - for manual releases:
   ```bash
   gh auth login
   ```

## Build Locally

### CLI Only

```bash
./scripts/build-cli.sh
# tarball: dist/linggen-cli-<slug>.tar.gz
# slugs: macos-aarch64, macos-x86_64, linux-arm64, linux-x86_64
```

### Server Only

```bash
cd backend
cargo build --release --bin linggen-server
```

### Tauri App

Requires sidecar built as `linggen-server`:

```bash
./deploy/build-tauri-app.sh    # copies linggen-server sidecar and builds DMG/app
```

## Local Testing

### Option 1: Local Manifest

```bash
./scripts/publish-cli-local.sh v0.0.1 /tmp/out
export LINGGEN_MANIFEST_URL=http://localhost:9000/manifest.json  # serve /tmp/out via a local http server
linggen install
```

### Option 2: Local Tarball (No Manifest)

```bash
bash ./install-cli.sh --local-path /path/to/dist/linggen-cli-<slug>.tar.gz
```

## Release Methods

### CI/CD Release (Automated)

**Workflow:** `.github/workflows/release.yml`

**Trigger:** Push a tag matching `v*` (e.g., `v0.2.0`)

**What it does:**

- Builds `linggen-server`, `linggen` CLI, packages tarballs
- Builds Tauri app (sidecar name: `linggen-server-<target-triple>`)
- Uploads artifacts to `linggen-releases` GitHub release
- CLI tarballs uploaded with three naming patterns:
  - Base name: `linggen-cli-<slug>.tar.gz` (e.g., `linggen-cli-macos-aarch64.tar.gz`)
  - Versioned: `linggen-cli-<slug>-v<version>.tar.gz` (e.g., `linggen-cli-macos-aarch64-v0.5.0.tar.gz`)
  - Latest: `linggen-cli-<slug>-latest.tar.gz` (e.g., `linggen-cli-macos-aarch64-latest.tar.gz`)
- Supported slugs: `macos-aarch64`, `linux-x86_64` (matches build matrix)
- Server tarballs: `linggen-server-<slug>.tar.gz`
- Emits `manifest.json` and `latest.json` for app/server updates

**To trigger:**

```bash
git tag v0.2.0
git push origin v0.2.0
```

Or use manual trigger via GitHub Actions UI with `workflow_dispatch`.

### Manual Release (Recommended: Script)

Use the automated script (handles builds, uploads, manifests, and replaces assets):

```bash
./scripts/manual-release.sh v0.2.0
```

What it does:

- Builds CLI and server
- Builds Tauri app (macOS only)
- Creates release if missing (draft), or uses existing
- Uploads base/versioned/latest tarballs, server, DMG
- Generates and uploads manifest/latest JSON
- Auto-deletes existing assets before upload

**Note:** Update `REPO` in the script to match your releases repo if needed.

## Installation Paths

### CLI Installation

1. **Install CLI first:**

   ```bash
   curl -fsSL https://linggen.dev/install-cli.sh | bash
   ```

   - Latest: `curl -fsSL https://linggen.dev/install-cli.sh | bash`
   - Specific version: `curl -fsSL https://linggen.dev/install-cli.sh | bash -s -- --version 0.5.0`
   - Local tarball: `bash install-cli.sh --local-path /path/to/linggen-cli-<slug>.tar.gz`

2. **Install app using CLI:**
   ```bash
   linggen install
   ```
   - Downloads and installs `Linggen.app` (macOS) or server (Linux) from manifest
   - Manifest URL: `https://github.com/linggen/linggen-releases/releases/latest/download/manifest.json`
   - Can be overridden with `LINGGEN_MANIFEST_URL` environment variable

**What gets installed:**

- **macOS**: CLI → `Linggen.app` (DMG) to `/Applications/`
- **Linux**: CLI → Server tarball + systemd unit (if available)

## Artifact Naming Requirements

The install script expects these naming patterns:

### For Latest Installations:

- `linggen-cli-<slug>-latest.tar.gz`
  - Example: `linggen-cli-macos-aarch64-latest.tar.gz`

### For Versioned Installations:

- `linggen-cli-<slug>-v<version>.tar.gz`
  - Example: `linggen-cli-macos-aarch64-v0.2.0.tar.gz`

### Platform Slugs:

- `macos-aarch64` (Apple Silicon)
- `macos-x86_64` (Intel Mac)
- `linux-x86_64` (Linux x86_64)
- `linux-arm64` (Linux ARM64)

### Manifest Keys

The CLI expects these keys in `manifest.json`:

- `cli-macos-universal` (required for macOS CLI)
- `server-macos-universal` (required for macOS server)
- `app-macos-dmg` (required for macOS app)
- `cli-linux-x86_64` (required for Linux CLI)
- `server-linux-x86_64` (required for Linux server)

## Verification

After uploading, verify the release:

```bash
gh release view v0.2.0 --repo linggen/linggen-releases
```

Test the install script:

```bash
curl -fsSL https://linggen.dev/install-cli.sh | bash
```

Then test app installation:

```bash
linggen install
```

## Troubleshooting

### "Release already exists"

If the release exists, you can still upload to it:

```bash
gh release upload v0.2.0 dist/linggen-cli-*.tar.gz --repo linggen/linggen-releases
```

The manual release script handles this automatically.

### "Permission denied"

Make sure you're authenticated:

```bash
gh auth status
gh auth login
```

### "File too large"

GitHub has a 2GB limit per file. If your DMG is too large, consider:

- Compressing it further
- Using GitHub LFS
- Splitting into multiple files

### CLI `--version` not working

If you see `error: unexpected argument '--version' found`, the CLI binary was built before version support was added. Rebuild and release a new version.

## Notes

- `install-cli.sh` is bash-only and supports:
  - `--version <ver>` (defaults to latest)
  - `--local-path <file://...tar.gz>`
- If using manifests for app/server, host them over HTTP/HTTPS (not file://).
- The manual release script automatically handles universal keys in manifests for CLI compatibility.
