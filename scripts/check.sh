#!/usr/bin/env bash
# Every check CI runs, runnable before a push: ./scripts/check.sh
#   ./scripts/check.sh          everything
#   ./scripts/check.sh rust     cargo test (unit: the ling binary and the engine
#                               crate) + generated TS types current + clippy count
#   ./scripts/check.sh system   the hermetic system tests (tests/system; cargo
#                               nextest; the engine hosts the memory). The
#                               memory test runs when the embedding model is in
#                               ~/.cache/huggingface (LINGGEN_SYSTEM_REQUIRE_EMBED=1:
#                               always, failing without the model)
#   ./scripts/check.sh ui       typecheck + lint
#   ./scripts/check.sh e2e      Playwright on the web UI (tests/e2e), each test
#                               on its own hermetic world; builds ui/dist and
#                               the debug engine first. Not part of `all`
#                               (~1 min with builds; needs Chromium)
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

# `npm ci` in $1 when its node_modules is missing or older than its lock.
npm_ready() {
  local installed="$1/node_modules/.package-lock.json"
  if [ ! -f "$installed" ] || [ "$1/package-lock.json" -nt "$installed" ]; then
    (cd "$1" && npm ci)
  fi
}

if [ "$what" = all ] || [ "$what" = rust ]; then
  # rust_embed needs the folder to exist even when the UI isn't built.
  mkdir -p ui/dist
  # Every target but tests/system, which runs as its own step below: the
  # binary's unit tests and the engine crate's (crates/linggen-engine).
  step cargo test --lib --bins --test rtc_link_bench
  # Memory (crates/linggen-memory), hosted by the engine.
  step cargo test -p linggen-memory
  # cargo test re-emits the ts-rs bindings; they must match what's committed.
  step ./scripts/gen-types.sh --check
  warnings=$(cargo clippy --all-targets --message-format=short 2>&1 | grep -c '^[^ ]*: warning' || true)
  echo "clippy: $warnings warnings (reported, not enforced)"
fi

if [ "$what" = all ] || [ "$what" = system ]; then
  mkdir -p ui/dist
  # The memory tests that embed run when the model is in
  # ~/.cache/huggingface, and say SKIPPED when it is not (a missing model
  # fails them with LINGGEN_SYSTEM_REQUIRE_EMBED=1). None is #[ignore]d.
  if cargo nextest --version >/dev/null 2>&1; then
    # On CI every failure is worth one run: do not stop at the first.
    step cargo nextest run --features test-support --test system ${CI:+--no-fail-fast}
  else
    echo "(cargo-nextest not installed — cargo install cargo-nextest --locked; using cargo test)"
    step cargo test --features test-support --test system
  fi
fi

if [ "$what" = all ] || [ "$what" = ui ]; then
  step npm_ready ui
  step bash -c 'cd ui && npx tsc --noEmit'
  step bash -c 'cd ui && npm run lint'
  step bash -c 'cd ui && node --test tests/*.test.mts'
fi

if [ "$what" = e2e ]; then
  step npm_ready ui
  step npm_ready tests/e2e
  step bash -c 'cd tests/e2e && npx playwright install chromium'
  step bash -c 'cd tests/e2e && npx playwright test'
fi

if [ "$what" = all ] || [ "$what" = js ]; then
  step node --test tests/shared/*.test.mjs
fi

[ "$fail" = 0 ] && echo "all checks passed" || echo "some checks failed"
exit "$fail"
