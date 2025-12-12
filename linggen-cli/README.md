# Linggen CLI

Standalone `linggen` binary that provides developer CLI commands and platform-aware install/update flows.

## TEST RUN

```bash
cargo run -p linggen-cli -- --version
cargo run -p linggen-cli -- install
```

## Build from source

```bash
# quick build
cargo build --release
./target/release/linggen --help

# or use helper
../scripts/build-cli.sh
# tarball emitted to dist/linggen-cli-<arch>-<os>.tar.gz
```

## Commands (current)

- `linggen start` / `linggen status` — check backend status
- `linggen index` — index a local directory (auto/full/incremental)
- `linggen serve` — ensure a local backend is running (spawns `linggen-server`)
- `linggen install` — platform-aware install (manifest-driven: macOS DMG, Linux tarball)
- `linggen update` — platform-aware update (manifest-driven)
- `linggen check` — local vs manifest comparison

## Backend expectation

This CLI talks to a running backend at `LINGGEN_API_URL` (default `http://127.0.0.1:8787`). It can also spawn `linggen-server` from PATH when needed.

## Roadmap

- Self-update for tarball installs
- Richer diagnostics and retries for install/update
- Shell completions generation

## Testing local publish

Use `../scripts/publish-cli-local.sh vX.Y.Z /tmp/out` to build a tarball and emit a minimal manifest. Then point the CLI at it:

```bash
export LINGGEN_MANIFEST_URL=file:///tmp/out/manifest.json
linggen install
```
