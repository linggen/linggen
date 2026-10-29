#!/bin/bash
# Build this checkout and restart the local engine on it, from inside a
# session the engine itself hosts (doc/background-tasks-spec.md § Engine
# restart).
#
#   scripts/restart-engine.sh            build ui (when stale) + ling, then restart
#   scripts/restart-engine.sh --no-build restart on the last release build
#   scripts/restart-engine.sh --status   how the last restart went
#
# A build error stops here with nothing touched. The restart itself is the
# engine's (`ling restart`): it waits for other turns, swaps the build in
# keeping ling.prev, rolls back if unhealthy, and every session it
# interrupted — the calling one included — continues afterwards.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BUILT="$REPO/target/release/ling"
HOME_DIR="${LINGGEN_HOME:-$HOME/.linggen}"

say() { echo "[restart $(date '+%H:%M:%S')] $*" >&2; }

build() {
  # shellcheck disable=SC1091
  . "$REPO/scripts/lib-common.sh"
  ensure_cargo
  # A release binary embeds ui/dist; rebuild it when any source is newer.
  if [ ! -f "$REPO/ui/dist/index.html" ] || \
     [ -n "$(find "$REPO/ui/src" "$REPO/ui/index.html" -newer "$REPO/ui/dist/index.html" -type f | head -1)" ]; then
    say "building ui"
    (cd "$REPO/ui" && npm run build >&2) || { say "ui build failed"; exit 1; }
  fi
  say "building ling (release)"
  (cd "$REPO" && cargo build --release --bin ling)
}

case "${1:-}" in
  --status)
    cat "$HOME_DIR/restart/status.json" 2>/dev/null || echo '{"state":"none"}'
    echo
    tail -20 "$HOME_DIR/restart/helper.log" 2>/dev/null || true ;;
  --no-build) exec "$BUILT" restart --binary "$BUILT" ;;
  "") build; exec "$BUILT" restart --binary "$BUILT" ;;
  *) echo "usage: $0 [--no-build|--status]" >&2; exit 2 ;;
esac
