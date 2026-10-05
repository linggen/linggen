#!/usr/bin/env bash
# Every check CI runs, runnable before a push: ./scripts/check.sh
#   ./scripts/check.sh          everything
#   ./scripts/check.sh rust     cargo test (unit) + generated TS types current + clippy count
#   ./scripts/check.sh system   the hermetic system tests (tests/system; cargo
#                               nextest; needs a ling-mem binary — LING_MEM_BIN,
#                               ../linggen-memory's release build, or PATH)
#   ./scripts/check.sh ui       typecheck + lint
#   ./scripts/check.sh js       node tests for the shared page helpers
#   ./scripts/check.sh live [flags]
#                               the live regression suite against the running
#                               engine on 9527 (scripts/live-check.sh; --no-model
#                               keeps it to the cases that call no model)
set -uo pipefail
cd "$(dirname "$0")/.."
if [ "${1:-}" = "live" ]; then
  shift
  exec ./scripts/live-check.sh "$@"
fi

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
  # Every target but tests/system, which runs as its own step below.
  step cargo test --bins --test rtc_link_bench
  # cargo test re-emits the ts-rs bindings; they must match what's committed.
  step ./scripts/gen-types.sh --check
  warnings=$(cargo clippy --all-targets --message-format=short 2>&1 | grep -c '^[^ ]*: warning' || true)
  echo "clippy: $warnings warnings (reported, not enforced)"
fi

if [ "$what" = all ] || [ "$what" = system ]; then
  mkdir -p ui/dist
  if cargo nextest --version >/dev/null 2>&1; then
    step cargo nextest run --test system
  else
    echo "(cargo-nextest not installed — cargo install cargo-nextest --locked; using cargo test)"
    step cargo test --test system
  fi
fi

if [ "$what" = all ] || [ "$what" = ui ]; then
  [ -d ui/node_modules ] || (cd ui && npm ci)
  step bash -c 'cd ui && npx tsc --noEmit'
  step bash -c 'cd ui && npm run lint'
  step bash -c 'cd ui && node --test tests/*.test.mts'
fi

if [ "$what" = all ] || [ "$what" = js ]; then
  step node --test tests/shared/*.test.mjs
fi

[ "$fail" = 0 ] && echo "all checks passed" || echo "some checks failed"
exit "$fail"
