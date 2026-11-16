#!/usr/bin/env bash
#
# Stamp the engine's version into the plugin: the VERSION file and the version
# in both manifests.
#
# The engine repo has one version ([workspace.package] in Cargo.toml) and the
# plugin in plugins/linggen is part of it — one tree that Claude Code and Codex
# both load directly, no per-host build output. scripts/sync-version.sh calls
# this with the release version; run alone, it stamps what Cargo.toml says.
#
#   ./scripts/plugin/build-plugin.sh [X.Y.Z]
#
# `scripts/plugin/plugin-check.sh` fails when the four disagree.

set -euo pipefail

cd "$(dirname "$0")/../.."

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' Cargo.toml | head -1)"
fi
VERSION="${VERSION#v}"
[ -n "$VERSION" ] || { echo "build-plugin: no version (give one, or fix [workspace.package] in Cargo.toml)" >&2; exit 1; }

ROOT="plugins/linggen"

printf 'v%s\n' "$VERSION" > "$ROOT/VERSION"
for manifest in "$ROOT/.claude-plugin/plugin.json" "$ROOT/.codex-plugin/plugin.json"; do
  # The top-level "version" line; the file keeps its layout.
  tmp="$(mktemp)"
  sed -E "s/^(  \"version\": )\"[^\"]*\"/\1\"$VERSION\"/" "$manifest" > "$tmp"
  cat "$tmp" > "$manifest"
  rm -f "$tmp"
  grep -q "^  \"version\": \"$VERSION\"" "$manifest" \
    || { echo "build-plugin: no top-level \"version\" line in $manifest" >&2; exit 1; }
done

echo "build-plugin: plugin stamped $VERSION (VERSION, .claude-plugin/plugin.json, .codex-plugin/plugin.json)"
