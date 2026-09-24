#!/usr/bin/env bash
# Every check CI runs, runnable before a push: ./scripts/check.sh
#   ./scripts/check.sh          everything
#   ./scripts/check.sh rust     cargo test + generated TS types current + clippy count
#   ./scripts/check.sh ui       typecheck + lint
#   ./scripts/check.sh js       node tests for the shared page helpers
set -uo pipefail
cd "$(dirname "$0")/.."

what=${1:-all}
fail=0
step() {
  echo "── $*"
  if ! "$@"; then
    echo "FAILED: $*"
    fail=1
  fi
}

if [ "$what" = all ] || [ "$what" = rust ]; then
  # rust_embed needs the folder to exist even when the UI isn't built.
  mkdir -p ui/dist
  step cargo test
  # cargo test re-emits the ts-rs bindings; they must match what's committed.
  step ./scripts/gen-types.sh --check
  warnings=$(cargo clippy --all-targets --message-format=short 2>&1 | grep -c '^[^ ]*: warning' || true)
  echo "clippy: $warnings warnings (reported, not enforced)"
fi

if [ "$what" = all ] || [ "$what" = ui ]; then
  [ -d ui/node_modules ] || (cd ui && npm ci)
  step bash -c 'cd ui && npx tsc --noEmit'
  step bash -c 'cd ui && npm run lint'
fi

if [ "$what" = all ] || [ "$what" = js ]; then
  step node --test tests/shared/*.test.mjs
fi

[ "$fail" = 0 ] && echo "all checks passed" || echo "some checks failed"
exit "$fail"
