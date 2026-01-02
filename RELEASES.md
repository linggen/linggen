# Linggen Release Process

This document describes how to create and publish new releases of Linggen with auto-update support.

## Overview

Linggen uses a two-repository release strategy:

- **Main repository** (`linggen`): Source code and development
- **Releases repository** (`linggen-releases`): Published binaries and update manifests

The auto-updater checks the `linggen-releases` repository for new versions and automatically downloads and installs updates.

## Version Management

Version numbers follow semantic versioning (MAJOR.MINOR.PATCH) and must be kept in sync across:

1. `frontend/src-tauri/Cargo.toml` - `version` field
2. `frontend/src-tauri/tauri.conf.json` - `version` field
3. `frontend/package.json` - `version` field

**Current version:** `0.2.0`

## Prerequisites

### One-Time Setup

1. **Generate Tauri Signing Keys** (only needed once):

   ```bash
   # Install Tauri CLI if not already installed
   npm install -g @tauri-apps/cli

   # Generate signing keypair
   tauri signer generate
   ```

   This will output:

   - **Private key**: Add to GitHub secrets as `TAURI_SIGNING_PRIVATE_KEY`
   - **Public key**: Already added to `tauri.conf.json` under `plugins.updater.pubkey`

2. **Configure GitHub Secrets**:

   In the main `linggen` repository, add these secrets:

   - `TAURI_SIGNING_PRIVATE_KEY` - The private key from step 1
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` - Password for the private key (if set)
   - `RELEASES_PAT` - Personal Access Token with permissions to create releases in `linggen-releases`

3. **Create linggen-releases Repository**:
   ```bash
   # Create a new public repository: linggen/linggen-releases
   # No code needed - releases will be published via GitHub Actions
   ```

## Release Workflow

### Step 1: Prepare Release

1. **Update version numbers** in all three files:

   ```bash
   # Update to new version (e.g., 0.3.0)
   # Edit frontend/src-tauri/Cargo.toml
   # Edit frontend/src-tauri/tauri.conf.json
   # Edit frontend/package.json
   ```

2. **Test locally**:

   ```bash
   # Build and test the app
   cd frontend
   npm run tauri:build

   # Test the built app
   open src-tauri/target/release/bundle/macos/Linggen.app
   ```

3. **Commit changes**:
   ```bash
   git add frontend/src-tauri/Cargo.toml \
           frontend/src-tauri/tauri.conf.json \
           frontend/package.json
   git commit -m "chore: bump version to 0.3.0"
   git push origin main
   ```

### Step 2: Create Release Tag

```bash
# Create and push a version tag
git tag v0.3.0
git push origin v0.3.0
```

This automatically triggers the GitHub Actions workflow that:

1. Builds for macOS (universal) and Linux (x86_64)
2. Signs the bundles with your private key
3. Generates update signatures
4. Creates updater manifest (`latest.json`)
5. Publishes to `linggen-releases` repository

### Step 3: Monitor Build

1. Go to GitHub Actions in the main repository
2. Watch the "Release Build and Publish" workflow
3. Verify it completes successfully

### Step 4: Verify Release

1. Check `linggen-releases` repository for new release
2. Verify these files are present:

   - `Linggen_0.3.0_universal.dmg` (macOS)
   - `Linggen_0.3.0_universal.dmg.tar.gz` (macOS update bundle)
   - `Linggen_0.3.0_universal.dmg.tar.gz.sig` (macOS signature)
   - `linggen_0.3.0_amd64.deb` (Linux)
   - `linggen_0.3.0_amd64.deb.tar.gz` (Linux update bundle)
   - `linggen_0.3.0_amd64.deb.tar.gz.sig` (Linux signature)
   - `latest.json` (Updater manifest)

3. Verify `latest.json` structure:
   ```json
   {
     "version": "0.3.0",
     "notes": "...",
     "pub_date": "2025-12-09T...",
     "platforms": {
       "darwin-universal": {
         "signature": "...",
         "url": "https://github.com/linggen/linggen-releases/releases/download/v0.3.0/..."
       },
       "linux-x86_64": {
         "signature": "...",
         "url": "https://github.com/linggen/linggen-releases/releases/download/v0.3.0/..."
       }
     }
   }
   ```

## Testing Auto-Updates

### Local Testing

1. **Install previous version**:

   ```bash
   # Download and install an older version from linggen-releases
   ```

2. **Launch the app**:

   ```bash
   open /Applications/Linggen.app
   ```

3. **Check for updates manually**:
   - Go to Settings → Software Update
   - Click "Check for Updates"
   - Verify new version is detected
   - Click "Install Update"
   - Verify app restarts with new version

### Automatic Update Check

The app automatically checks for updates on startup (configured in `tauri.conf.json`):

```json
"updater": {
  "active": true,
  "dialog": true,
  ...
}
```

When `dialog: true`, users see a native dialog when an update is available.

## Update Endpoints

The updater checks this URL for updates:

```
https://github.com/linggen/linggen-releases/releases/latest/download/latest.json
```

To test different update channels in the future, you can:

1. Create channel-specific manifests: `beta.json`, `stable.json`
2. Update `tauri.conf.json` endpoints array
3. Add environment-based selection logic

## Troubleshooting

### Build Failures

**Problem**: GitHub Action fails during build

- **Check**: Rust dependencies and version compatibility
- **Check**: Node.js version (requires 20.x)
- **Check**: Tauri dependencies installed correctly

**Problem**: Signing fails

- **Check**: `TAURI_SIGNING_PRIVATE_KEY` secret is set correctly
- **Check**: Public key in `tauri.conf.json` matches private key
- **Check**: Private key password (if used) is set in secrets

### Update Check Failures

**Problem**: "Failed to check for updates" in app

- **Check**: `linggen-releases` repository is public
- **Check**: `latest.json` is accessible at the endpoint URL
- **Check**: JSON format is valid

**Problem**: Update found but download fails

- **Check**: Asset URLs in `latest.json` are correct
- **Check**: `.tar.gz` and `.sig` files exist in release
- **Check**: Network connectivity

**Problem**: Signature verification fails

- **Check**: Public key in `tauri.conf.json` matches the private key used for signing
- **Check**: `.sig` files are correctly generated and uploaded

### Version Mismatch

**Problem**: App doesn't detect update even though new version exists

- **Check**: Version in `latest.json` is higher than current app version
- **Check**: Version format is consistent (no leading 'v' in JSON)
- **Check**: Platform-specific entry exists in `latest.json`

## Manual Release (Alternative)

If you need to create a release manually without GitHub Actions:

### Linux (Multi-Arch)

To build for both x86_64 and ARM64 Linux using Docker Buildx:

```bash
./deploy/build-linux-multiarch.sh
```

This will produce `.deb` and `.AppImage` files in `dist/linux/` for both architectures.

### Local Build (Single Architecture)

1. **Build locally**:

   ```bash
   ./deploy/build-tauri-app.sh
   ```

2. **Sign bundles** (requires private key):

   ```bash
   tauri signer sign \
     dist/macos/Linggen.app \
     --private-key ~/.tauri/private-key.pem
   ```

3. **Create update bundles**:

   ```bash
   # macOS
   cd dist/macos
   tar czf Linggen.dmg.tar.gz Linggen.dmg
   tauri signer sign Linggen.dmg.tar.gz --private-key ~/.tauri/private-key.pem

   # Linux
   cd dist/linux
   tar czf linggen.deb.tar.gz linggen_*.deb
   tauri signer sign linggen.deb.tar.gz --private-key ~/.tauri/private-key.pem
   ```

4. **Create `latest.json` manually** and upload to `linggen-releases`

## Security Notes

- **Never commit private keys** to the repository
- Keep `TAURI_SIGNING_PRIVATE_KEY` secure in GitHub secrets
- The public key in `tauri.conf.json` is safe to commit
- Update signatures ensure users only install authentic updates
- Users must have the matching public key compiled into their app

## Future Enhancements

- [ ] Add Windows support to build matrix
- [ ] Implement update channels (stable, beta, nightly)
- [ ] Add automatic rollback on update failure
- [ ] Create release notes generation from git log
- [ ] Add update size information to manifest
- [ ] Implement delta updates for smaller downloads
