#!/bin/sh
# Run the engine's tests against a scratch home, never the real one.
#
#   scripts/scratch-test.sh              unit tests (--lib --bins)
#   scripts/scratch-test.sh system       the system tests (--test system)
#   scripts/scratch-test.sh <cargo test arguments…>
#
# The engine's tests and any engine they start read and write their home
# folder: with the real one they would open the owner's own memory store and
# sessions. This points HOME and Linggen's folders at a scratch folder under
# ~/.cache (named after this checkout, so worktrees do not share one) and
# keeps cargo and rustup where they are. One plain command, so an agent's
# session can run it without a variable in its command line.
#
# For the system tests the machine's embedding model cache
# (~/.cache/huggingface) is linked into the scratch home, as the release
# gate does: the tests that embed clone the model out of it into their own
# world (read only — nothing writes through the link), so they run for real.
# A test that cannot run here (no model on this machine) names itself in a
# file; the names are listed as SKIPPED at the end, never silently green.
set -eu
real_home="$HOME"
here="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$real_home/.cache/linggen-test-home/$(basename "$here")"
mkdir -p "$scratch/.linggen/skills" "$scratch/.cache"
models="$scratch/.cache/huggingface"
# Only the system tests see the link; unit tests load nothing through it.
if [ -L "$models" ]; then rm "$models"; fi
case "${1:-}" in
  "") set -- --lib --bins ;;
  system)
    shift; set -- --features test-support --test system "$@"
    if [ -d "$real_home/.cache/huggingface" ]; then ln -s "$real_home/.cache/huggingface" "$models"; fi
    ;;
esac
skipped="$scratch/skipped-tests.txt"
: > "$skipped"
cd "$here"
status=0
env HOME="$scratch" \
  CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
  RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
  LINGGEN_HOME="$scratch/.linggen" LINGGEN_DATA_DIR="$scratch/.linggen" \
  LINGGEN_NO_BONJOUR=1 LINGGEN_TEST_SKIPPED="$skipped" \
  nice -n 10 cargo test -j "${TEST_JOBS:-6}" "$@" || status=$?
if [ -s "$skipped" ]; then
  echo
  echo "SKIPPED — these passed on nothing here:"
  sort -u "$skipped" | sed 's/^/  /'
fi
exit "$status"
