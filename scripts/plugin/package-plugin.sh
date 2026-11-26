#!/usr/bin/env bash
#
# Package the plugin marketplace as one tarball — the bundle a machine
# without git installs from (linggen.dev/install-plugin.sh):
#
#   ./scripts/plugin/package-plugin.sh <out.tar.gz>
#
# The tarball's root is a marketplace both hosts read as a local directory:
#   .claude-plugin/marketplace.json   Claude Code (source ./plugins/linggen)
#   .agents/plugins/marketplace.json  Codex
#   plugins/linggen/                  the plugin tree, minus its eval suite
# Writes <out.tar.gz>.sha256 beside it.
set -euo pipefail

out="${1:?usage: package-plugin.sh <out.tar.gz>}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
case "$out" in /*) ;; *) out="$PWD/$out" ;; esac

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/.claude-plugin" "$stage/.agents/plugins" "$stage/plugins"
cp "$root/.claude-plugin/marketplace.json" "$stage/.claude-plugin/"
cp "$root/.agents/plugins/marketplace.json" "$stage/.agents/plugins/"
rsync -a --exclude evals-isolated --exclude .DS_Store "$root/plugins/linggen" "$stage/plugins/"

[ -f "$stage/plugins/linggen/skills/linggen/scripts/install.sh" ] \
  || { echo "package-plugin: vendored install.sh is missing" >&2; exit 1; }

# Both manifests must point inside the bundle, or a host would reach for git.
grep -q '"source": "./plugins/linggen"' "$stage/.claude-plugin/marketplace.json" \
  || { echo "package-plugin: .claude-plugin/marketplace.json must use source ./plugins/linggen" >&2; exit 1; }

mkdir -p "$(dirname "$out")"
COPYFILE_DISABLE=1 tar -C "$stage" -czf "$out" .
(cd "$(dirname "$out")" && shasum -a 256 "$(basename "$out")" >"$(basename "$out").sha256")
echo "package-plugin: $out ($(du -h "$out" | awk '{print $1}'))"
