#!/bin/bash
# Run once inside a fresh `git worktree` of this repo, before its first build:
#
#   scripts/worktree-setup.sh          the built web UI, the avatar clips, and a head start for the build
#   scripts/worktree-setup.sh --ui     also ui/node_modules, for a worktree that will build the web UI
#
# Everything is a copy-on-write clone of the main checkout's own files (APFS `cp -c`: about a second, no disk
# used until a file changes) — never a link, so a build here can never write into the main checkout.
#
# - ui/dist is not in git and the engine embeds it; the avatar's raw clips (ui/anim-src) and the clips baked
#   from them (ui/public/anim-baked) are not in git either — `npm run build` bakes anim-src into anim-baked,
#   and a worktree that only needs the build can live on the cloned anim-baked alone.
# - target/fast: with the main checkout's libraries already compiled, the first build here compiles our own code
#   only — 87 s instead of 170 s (measured 2026-11-15). The folder is this worktree's own from then on: no
#   shared lock, no shared binary.
set -euo pipefail

HERE="$(git rev-parse --show-toplevel)"
MAIN="$(cd "$(git rev-parse --git-common-dir)/.." && pwd)"
say() { echo "worktree-setup: $*"; }
[ "$HERE" != "$MAIN" ] || { say "this is the main checkout — nothing to do"; exit 0; }

# clone <path relative to the checkout> — only when the main checkout has it and this one does not.
clone() {
  local from="$MAIN/$1" to="$HERE/$1"
  [ -e "$from" ] || { say "$1: the main checkout has none (build it there first)"; return 0; }
  [ ! -e "$to" ] || { say "$1: already here"; return 0; }
  mkdir -p "$(dirname "$to")"
  if cp -cRp "$from" "$to" 2>/dev/null; then say "$1: cloned"; else rm -rf "$to"; say "$1: no copy-on-write on this disk — skipped"; fi
}

clone ui/dist
clone ui/anim-src
clone ui/public/anim-baked
clone target/fast
# The test build too: without it the first `cargo test` here compiles every library again (ten minutes and more
# when two worktrees do it at once — seen 2026-11-15).
clone target/debug
[ "${1:-}" != "--ui" ] || clone ui/node_modules
