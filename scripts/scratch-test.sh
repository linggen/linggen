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
set -eu
real_home="$HOME"
here="$(cd "$(dirname "$0")/.." && pwd)"
scratch="$real_home/.cache/linggen-test-home/$(basename "$here")"
mkdir -p "$scratch/.linggen/skills"
case "${1:-}" in
  "") set -- --lib --bins ;;
  system) shift; set -- --test system "$@" ;;
esac
cd "$here"
exec env HOME="$scratch" \
  CARGO_HOME="${CARGO_HOME:-$real_home/.cargo}" \
  RUSTUP_HOME="${RUSTUP_HOME:-$real_home/.rustup}" \
  LINGGEN_HOME="$scratch/.linggen" LINGGEN_DATA_DIR="$scratch/.linggen" \
  LINGGEN_NO_BONJOUR=1 \
  nice -n 10 cargo test -j "${TEST_JOBS:-6}" "$@"
