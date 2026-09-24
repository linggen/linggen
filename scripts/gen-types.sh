#!/usr/bin/env bash
# Regenerate ui/src/types/generated/ from the Rust wire types (ts-rs, under
# `cargo test`), then fail if the committed copies were out of date.
#   ./scripts/gen-types.sh          regenerate + check
#   ./scripts/gen-types.sh --check  check only (after a `cargo test` already ran)
set -euo pipefail
cd "$(dirname "$0")/.."

out=ui/src/types/generated
if [ "${1:-}" != "--check" ]; then
  mkdir -p ui/dist
  rm -rf "$out"
  cargo test export_bindings --quiet >/dev/null
fi

if ! git diff --exit-code --stat -- "$out" || [ -n "$(git ls-files --others --exclude-standard -- "$out")" ]; then
  echo "ui/src/types/generated is stale — run ./scripts/gen-types.sh and commit the result"
  exit 1
fi
echo "generated types are current"
