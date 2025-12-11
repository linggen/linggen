# Tauri App Signing Setup

This guide shows how to set up code signing for Tauri app updates.

## Why Sign Updates?

Signing ensures that users only install authentic updates from you, preventing malicious actors from distributing tampered versions.

## Setup (One-Time)

### 1. Install Tauri CLI

```bash
npm install -g @tauri-apps/cli
```

### 2. Generate Signing Keys

```bash
tauri signer generate --write-keys
```

This creates:

- **Private key**: `~/.tauri/linggen.key` (keep this secret!)
- **Public key**: Already configured in `frontend/src-tauri/tauri.conf.json`

**Important**: Back up your private key securely. If lost, you cannot sign updates and users will need to reinstall.

### 3. Secure Your Private Key

```bash
# Set restrictive permissions
chmod 600 ~/.tauri/linggen.key

# Optional: Set password protection
tauri signer generate --write-keys --password "your-secure-password"
```

## Usage

### Manual Releases

When running `./scripts/manual-release.sh`, the script will automatically sign the DMG if it finds:

**Option A: Environment variable (recommended for CI)**

```bash
export TAURI_PRIVATE_KEY="$(cat ~/.tauri/linggen.key)"
export TAURI_PRIVATE_KEY_PASSWORD="your-password"  # if key is password-protected
./scripts/manual-release.sh v0.3.0
```

**Option B: Key file (easier for local releases)**

```bash
# Just run the script - it auto-detects ~/.tauri/linggen.key
./scripts/manual-release.sh v0.3.0
```

### GitHub Actions (CI)

For automated releases via GitHub Actions:

1. Go to your repository → Settings → Secrets and variables → Actions
2. Add secrets:
   - `TAURI_SIGNING_PRIVATE_KEY`: Content of `~/.tauri/linggen.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: Your key password (if set)

The workflow in `.github/workflows/release.yml` already uses these secrets.

## Verification

After signing, the script:

1. Creates `Linggen_X.Y.Z_aarch64.dmg.sig` (signature file)
2. Uploads it to GitHub releases
3. Includes signature in `latest.json` for the updater

Users' Tauri apps automatically verify signatures using the public key in `tauri.conf.json`.

## Troubleshooting

### "No signing key found"

**Cause**: Script couldn't find `~/.tauri/linggen.key` or `TAURI_PRIVATE_KEY` env var

**Fix**:

```bash
# Generate key if you don't have one
tauri signer generate --write-keys

# Or set environment variable
export TAURI_PRIVATE_KEY="$(cat /path/to/your/key.key)"
```

### "Signature verification failed" (users see this)

**Cause**: Public key mismatch or corrupted signature

**Fix**:

1. Ensure `tauri.conf.json` has the correct public key matching your private key
2. Re-generate and re-sign if you regenerated keys

### Password Issues

If your key is password-protected but script fails:

```bash
# Set password env var
export TAURI_PRIVATE_KEY_PASSWORD="your-password"
./scripts/manual-release.sh v0.3.0
```

## Security Best Practices

✅ **Do:**

- Keep private key secure (never commit to git)
- Use password protection for extra security
- Back up your key securely
- Use environment variables in CI/CD
- Rotate keys periodically (requires user reinstall)

❌ **Don't:**

- Share your private key
- Commit keys to version control
- Use the same key across different projects
- Leave keys in plain text on shared systems

## Disable Signing (Not Recommended)

If you want to disable signature verification entirely (e.g., for development):

Edit `frontend/src-tauri/tauri.conf.json`:

```json
"updater": {
  "pubkey": ""  // Empty string disables verification
}
```

⚠️ **Warning**: This allows anyone to distribute malicious updates to your users.
