#!/bin/bash
# Build this checkout and restart the local engine on it, safely from inside
# a session the engine itself hosts (a Linggen Claude session, `ling claude`).
#
#   scripts/restart-engine.sh            build, then swap + restart (detached)
#   scripts/restart-engine.sh --no-build swap + restart the last release build
#   scripts/restart-engine.sh --status   print the last restart's outcome
#
# The build runs in the foreground, so a compile error stops here with
# nothing touched. The swap and restart run in a new process session: the
# caller may be a child of the engine being stopped and die with it, while
# this script carries on. It keeps the running binaries as `ling.prev`,
# replaces both installs atomically, stops the process listening on the
# port, starts `ling --web`, and waits for /api/health from a new process.
# If none answers, it puts `ling.prev` back and starts that. The outcome is
# written to $STATUS (read it with --status after reconnecting).
set -euo pipefail

PORT="${LINGGEN_PORT:-9527}"
LOGS="${LINGGEN_HOME:-$HOME/.linggen}/logs"
STATUS="$LOGS/restart-status.json"
LOCK="$LOGS/restart.lock.d"
# Overridable so the script can be exercised on a scratch port and copies.
BIN="${RESTART_BIN:-/usr/local/bin/ling}"
APP_BIN="${RESTART_APP_BIN:-/Applications/Linggen.app/Contents/MacOS/ling}"
HEALTH_WAIT="${RESTART_HEALTH_WAIT:-60}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
BUILT="${RESTART_BUILT:-$REPO/target/release/ling}"
mkdir -p "$LOGS"

say() { echo "[restart $(date '+%H:%M:%S')] $*" >&2; }

status() {
  printf '{"state":"%s","detail":"%s","pid":"%s","at":"%s","log":"%s"}\n' \
    "$1" "$2" "${3:-}" "$(date '+%F %T')" "${LOG:-}" > "$STATUS"
}

# Empty, not a failure, when nothing listens (lsof exits 1 then).
listener() { lsof -nP -t -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null | head -1 || true; }

healthy() { curl -fsS -m 2 "http://127.0.0.1:$PORT/api/health" >/dev/null 2>&1; }

# ── Phase 2 (detached): swap, stop, start, verify, roll back ──────────────
stop_engine() {
  local pid; pid="$(listener)"
  [ -z "$pid" ] && return 0
  say "stopping pid $pid"
  kill -TERM "$pid" 2>/dev/null || true
  for _ in $(seq 1 50); do kill -0 "$pid" 2>/dev/null || return 0; sleep 0.2; done
  say "pid $pid ignored TERM; killing"
  kill -KILL "$pid" 2>/dev/null || true
  sleep 0.5
}

# Start `ling --web`, then wait for a healthy listener other than $1.
start_engine() {
  local old="$1"
  say "starting $BIN --web"
  nohup "$BIN" --web --port "$PORT" >> "$LOGS/engine-restart.log" 2>&1 < /dev/null &
  for _ in $(seq 1 $((HEALTH_WAIT * 2))); do
    local pid; pid="$(listener)"
    if [ -n "$pid" ] && [ "$pid" != "$old" ] && healthy; then
      echo "$pid"
      return 0
    fi
    sleep 0.5
  done
  return 1
}

install_binary() { # src dst — fresh inode, never a write over a running file
  cp "$1" "$2.new"
  mv -f "$2.new" "$2"
}

swap_and_restart() {
  # One restart at a time; a lock left by a run killed mid-way expires.
  if [ -d "$LOCK" ] && [ -n "$(find "$LOCK" -maxdepth 0 -mmin +5)" ]; then rmdir "$LOCK"; fi
  if ! mkdir "$LOCK" 2>/dev/null; then
    say "another restart is running"; status failed "another restart is running"; exit 1
  fi
  trap 'rmdir "$LOCK" 2>/dev/null || true' EXIT
  local old; old="$(listener)"
  status restarting "swapping binaries" "$old"
  cp "$BIN" "$BIN.prev"
  [ -f "$APP_BIN" ] && cp "$APP_BIN" "$APP_BIN.prev"
  install_binary "$BUILT" "$BIN"
  [ -f "$APP_BIN" ] && install_binary "$BUILT" "$APP_BIN"
  say "installed $("$BIN" --version)"
  stop_engine
  local pid
  if pid="$(start_engine "$old")"; then
    say "healthy on pid $pid"
    status ok "new build healthy" "$pid"
    return 0
  fi
  say "no healthy engine in ${HEALTH_WAIT}s; rolling back"
  stop_engine
  install_binary "$BIN.prev" "$BIN"
  [ -f "$APP_BIN.prev" ] && install_binary "$APP_BIN.prev" "$APP_BIN"
  if pid="$(start_engine "")"; then
    status rolled_back "new build never became healthy; previous binary restored" "$pid"
  else
    status failed "rollback did not come up either; see $LOGS/engine-restart.log"
  fi
  exit 1
}

# ── Phase 1 (foreground): build, then hand off ────────────────────────────
build() {
  # shellcheck disable=SC1091
  . "$REPO/scripts/lib-common.sh"
  ensure_cargo
  # A release binary embeds ui/dist; rebuild it when any source is newer.
  if [ ! -f "$REPO/ui/dist/index.html" ] || \
     [ -n "$(find "$REPO/ui/src" "$REPO/ui/index.html" -newer "$REPO/ui/dist/index.html" -type f | head -1)" ]; then
    say "building ui"
    (cd "$REPO/ui" && npm run build >> "$LOGS/ui-build.log" 2>&1) \
      || { say "ui build failed; see $LOGS/ui-build.log"; exit 1; }
  fi
  say "building ling (release)"
  (cd "$REPO" && cargo build --release --bin ling)
}

detach() {
  LOG="$LOGS/restart-$(date '+%Y%m%d-%H%M%S').log"
  status starting "handing off to a detached restart" "$(listener)"
  /usr/bin/python3 - "$0" "$LOG" <<'PY'
import os, sys
script, log = sys.argv[1], sys.argv[2]
if os.fork():
    sys.exit(0)
os.setsid()
if os.fork():
    sys.exit(0)
out = os.open(log, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644)
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
os.dup2(out, 1)
os.dup2(out, 2)
os.environ["RESTART_LOG"] = log
os.execv("/bin/bash", ["/bin/bash", script, "--swap"])
PY
  say "restart handed off; log: $LOG"
  say "this session may drop now; afterwards run: $0 --status"
}

case "${1:-}" in
  --status) cat "$STATUS" 2>/dev/null || echo '{"state":"none"}' ;;
  --swap) LOG="${RESTART_LOG:-}"; swap_and_restart ;;
  --no-build) [ -x "$BUILT" ] || { say "no build at $BUILT"; exit 1; }; detach ;;
  "") build; detach ;;
  *) echo "usage: $0 [--no-build|--status]" >&2; exit 2 ;;
esac
