## Deploy Guide

This repo now ships separate artifacts:

- `linggen` (standalone CLI)
- `linggen-server` (backend/server; Tauri sidecar)
- `Linggen.app` (macOS desktop)

### Build locally

- CLI only:
  ```bash
  ./scripts/build-cli.sh
  # tarball: dist/linggen-cli-<slug>.tar.gz (slugs: macos-aarch64, macos-x86_64, linux-arm64, linux-x86_64)
  ```
- Server only:
  ```bash
  cd backend
  cargo build --release --bin linggen-server
  ```
- Tauri app (requires sidecar built as `linggen-server`):
  ```bash
  ./deploy/build-tauri-app.sh    # copies linggen-server sidecar and builds DMG/app
  ```

### Local publish test

Option 1: Local manifest

```bash
./scripts/publish-cli-local.sh v0.0.1 /tmp/out
export LINGGEN_MANIFEST_URL=http://localhost:9000/manifest.json  # serve /tmp/out via a local http server
linggen install
```

Option 2: Local tarball (no manifest)

```bash
bash ./install-cli.sh --local-path /path/to/dist/linggen-cli-<slug>.tar.gz
```

### CI release (overview)

- Workflow: `.github/workflows/release.yml`
- On tag push (v\*):
  - Build `linggen-server`, `linggen` CLI, package tarballs.
  - Build Tauri app (sidecar name: `linggen-server-<target-triple>`).
  - Upload artifacts to `linggen-releases` GitHub release.
  - Upload “latest” CLI tarballs expected by installer:
    - Base name: `linggen-cli-<slug>.tar.gz` (e.g., `linggen-cli-macos-aarch64.tar.gz`)
    - Versioned: `linggen-cli-<slug>-v<version>.tar.gz` (e.g., `linggen-cli-macos-aarch64-v0.5.0.tar.gz`)
    - Latest: `linggen-cli-<slug>-latest.tar.gz` (e.g., `linggen-cli-macos-aarch64-latest.tar.gz`)
  - Supported slugs: `macos-aarch64`, `linux-x86_64` (matches build matrix)
  - Server tarballs: `linggen-server-<slug>.tar.gz`
  - Emit `manifest.json` and `latest.json` for app/server updates.

### Install paths (current)

- CLI:
  - Latest: `curl -fsSL https://linggen.dev/install-cli.sh | bash`
  - Specific version: `curl -fsSL https://linggen.dev/install-cli.sh | bash -s -- --version 0.5.0`
  - Local tarball: `bash install-cli.sh --local-path /path/to/linggen-cli-<slug>.tar.gz`
- Desktop app (macOS): DMG installs `Linggen.app` (bundles `linggen-server` sidecar).
- Server (Linux/macOS): tarball + systemd/manual; `linggen install` can be wired to a manifest for runtime/app if desired.

### Notes

- `install-cli.sh` is bash-only and supports:
  - `--version <ver>` (defaults to latest)
  - `--local-path <file://...tar.gz>`
- If using manifests for app/server, host them over HTTP/HTTPS (not file://).
