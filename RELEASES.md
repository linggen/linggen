# Linggen Release Process

This document describes how to create and publish new releases of Linggen.

## Overview

Linggen uses a single repository release strategy:

- **Main repository** (`linggen`): Source code, development, and published releases.

The CLI handles updates by checking the `linggen` repository for new versions and automatically downloading and installing updates.

## Version Management

Version numbers follow semantic versioning (MAJOR.MINOR.PATCH) and must be kept in sync across:

1. `linggen-cli/Cargo.toml` - `version` field
2. `frontend/package.json` - `version` field
3. `backend/Cargo.toml` - workspace version

**Current version:** `0.6.1`

## Prerequisites

### GitHub CLI
Ensure you have the `gh` CLI installed and authenticated.

## Release Workflow

### Step 1: Prepare Release

1. **Update version numbers** using the sync script:

   ```bash
   ./scripts/sync-version.sh v0.7.0
   ```

2. **Test locally**:

   ```bash
   # Build everything
   ./scripts/build.sh v0.7.0
   ```

3. **Commit changes**:
   ```bash
   git add .
   git commit -m "chore: bump version to 0.7.0"
   git push origin main
   ```

### Step 2: Create Release Tag

```bash
git tag v0.7.0
git push origin v0.7.0
```

### Step 3: Publish Release

To build, sign, and upload to GitHub in one go:

```bash
./scripts/release.sh v0.7.0
```

Use `--draft` if you want to inspect the release before publishing:

```bash
./scripts/release.sh v0.7.0 --draft
```

### Step 4: Verify Release

1. Check `linggen` repository for new release
2. Verify these files are present:

   - `linggen-cli-macos-aarch64.tar.gz` (macOS CLI)
   - `linggen-server-macos.tar.gz` (macOS Server)
   - `linggen-cli-linux-x86_64.tar.gz` (Linux CLI)
   - `linggen-server-linux-x86_64.tar.gz` (Linux Server)
   - `manifest.json` (Update manifest)

## Script Architecture

- **`scripts/lib-common.sh`**: Shared helpers for platform detection and signing.
- **`scripts/build.sh`**: Master build orchestrator.
- **`scripts/build-mac.sh`**: macOS-specific build logic.
- **`scripts/build-linux.sh`**: Linux-specific build logic (Docker).
- **`scripts/release.sh`**: Orchestrates packaging and GitHub publishing.
- **`scripts/sync-version.sh`**: Ensures version consistency across all files.

## Security Notes

- **Never commit private keys** to the repository.
- Keep signing keys secure.
- Update signatures ensure users only install authentic updates.
