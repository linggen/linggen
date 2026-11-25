#!/usr/bin/env bash
# Release gate — Memory V2 on a scratch engine on this Mac, no VM
# (vm/notes.sh holds the checks; the VM runs the same ones on a real install
# and a real upgrade). Three homes, each its own engine on a free port:
#
#   fresh      no config: ends on v1 (the default; v2 is opt-in in 2.7), nothing moved
#   old, v1    a home with v1 rows and `[memory]` written as v1: stays v1
#   old, dflt  the same home with the section taken out (the default, as an
#              upgrade from 2.6.1 leaves it): stays v1, nothing moved
#   old, v2    the same home with `[memory]` written as v2: the rows move into
#              notes once; a second start moves nothing again
#
# Usage: scripts/gate/notes-local.sh <ling binary>
# Prints the gate's RESULT lines (vm/common.sh); the last line is GATE_EXIT.
# Never touches ~/.linggen, 9527 or 9528: every home is a folder under
# ~/.cache/linggen-gate-notes, the engine is started and stopped by its pid,
# and nothing reaches the network (dead proxy, no telemetry).
#
# The old rows are written through the engine's own memory (they need its
# embedding model: the machine's ~/.cache/huggingface, linked in, as
# scratch-test.sh does). Without it the seeded half is a GAP.
#
# DOOR_TOOL_NAMES (comma-separated) is the catalogue's list, as gate.sh hands
# it to the VMs; when unset it is read from the plugin evals' _tools.json.

set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
LING_SRC="${1:?usage: notes-local.sh <ling binary>}"
LING_SRC="$(cd "$(dirname "$LING_SRC")" && pwd)/$(basename "$LING_SRC")"
[ -x "$LING_SRC" ] || { echo "notes-local: $LING_SRC is not executable" >&2; exit 2; }

REAL_HOME="$HOME"
S="$REAL_HOME/.cache/linggen-gate-notes/$(date +%Y%m%d-%H%M%S)-$$"
mkdir -p "$S"
: "${DOOR_TOOL_NAMES:=$(jq -r '[.tools[].name] | join(",")' "$REPO/plugins/linggen/evals-isolated/mocks/linggen/_tools.json")}"
export DOOR_TOOL_NAMES

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

# Each home is its own HOME for the helpers below: common.sh reads $HOME for
# ~/.linggen and ~/gate, so it is sourced afresh per home, in a subshell.
home_env() { # home port → the engine's environment, one per line
  cat <<EOF
HOME=$1
LINGGEN_HOME=$1/.linggen
LINGGEN_CONFIG=$1/.linggen/config/linggen.runtime.toml
TMPDIR=$1/tmp
LINGGEN_NO_TELEMETRY=1
LING_MEM_NO_TELEMETRY=1
LINGGEN_NO_BONJOUR=1
LINGGEN_SITE_URL=http://127.0.0.1:9
HTTP_PROXY=http://127.0.0.1:9
HTTPS_PROXY=http://127.0.0.1:9
http_proxy=http://127.0.0.1:9
https_proxy=http://127.0.0.1:9
NO_PROXY=127.0.0.1,localhost
HF_HUB_OFFLINE=1
NO_COLOR=1
LANG=en_US.UTF-8
TZ=UTC
EOF
}

# One scenario: its own home, port and engine. $1 = name; the rest of the
# function body is `scenario_<name>`, run with common.sh and notes.sh loaded.
run_scenario() { # name
  local name="$1" home="$S/$1" port
  port="$(free_port)"
  mkdir -p "$home/.linggen/config" "$home/.linggen/skills" "$home/.local/bin" "$home/.cache" "$home/tmp" "$home/gate" "$home/work"
  [ -d "$REAL_HOME/.cache/huggingface" ] && ln -sfn "$REAL_HOME/.cache/huggingface" "$home/.cache/huggingface"
  ln -sf "$LING_SRC" "$home/.local/bin/ling"
  (
    while IFS= read -r line; do export "$line"; done < <(home_env "$home")
    export GATE_ENGINE_PORT="$port" MIRROR=none GATE_SOURCE=local
    cd "$HERE/vm" || exit 1
    . ./common.sh
    . ./takeover.sh
    . ./notes.sh
    WORK="$home/work"
    # `ling` as a person's daemon, but on this port, with no browser: by pid.
    engine_up() { # log-name
      # exec: the subshell's pid is the engine's, so `kill` stops that one.
      (cd "$WORK" && exec "$home/.local/bin/ling" --web --port "$EP" --root "$WORK") >"$OUT/engine-$1.log" 2>&1 </dev/null &
      echo $! >"$OUT/engine.pid"
      wait_http "http://127.0.0.1:$EP/api/health" 120
    }
    engine_down() {
      local pid i; pid="$(cat "$OUT/engine.pid" 2>/dev/null)"
      [ -n "$pid" ] || return 0
      kill "$pid" 2>/dev/null
      for ((i = 0; i < 30; i++)); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
      kill -9 "$pid" 2>/dev/null; { wait "$pid"; } 2>/dev/null; rm -f "$OUT/engine.pid"
      nothing_on "$EP" || { fail "notes ($name): the scratch engine stopped" "$PORT_DETAIL"; return 1; }
    }
    trap 'engine_down; printf "GATE_EXIT\t%s\n" "$?"' EXIT
    # The gate's own config (a fake model nothing calls) on this port, and
    # `[memory]` as the scenario writes it.
    write_config() { # extra toml
      sed -e "s#127.0.0.1:9527#127.0.0.1:$EP#" -e 's#{{MODEL_URL}}#http://127.0.0.1:9#' "$HERE/vm/gate.runtime.toml" >"$LG/config/linggen.runtime.toml"
      printf '\n%s\n' "${1:-}" >>"$LG/config/linggen.runtime.toml"
    }
    "scenario_$name"
  )
}
# The subshell above prints its own GATE_EXIT; only the last one is read.

scenario_fresh() {
  write_config ""
  engine_up fresh
  check "notes (fresh home): the engine starts on its own port" $? "no health on $EP — $(tail -3 "$OUT/engine-fresh.log")" "port $EP"
  notes_fresh "fresh install (scratch engine)"
}

# Rows written the way the baseline wrote them: through the engine's /mem
# (the same five as inplace.sh: three long-term, one staged, one core).
seed_v1_rows() {
  local i ok=0 stamp="local-$(date +%s)" tries=0
  tk_ids "$ENGINE_MEM" >"$TK_ALL"
  # The first add waits on the embedding model; try until it answers.
  until tk_add "$ENGINE_MEM" "baseline engine" semantic "Gate in-place long-term row 0 ($stamp): Alex keeps the harbor ledger in a green notebook."; do
    tries=$((tries + 1)); [ "$tries" -ge 40 ] && return 1; sleep 3
  done
  ok=1
  for i in 1 2; do
    tk_add "$ENGINE_MEM" "baseline engine" semantic "Gate in-place long-term row $i ($stamp): Alex keeps the harbor ledger in a green notebook." && ok=$((ok + 1))
  done
  tk_add "$ENGINE_MEM" "baseline engine" staged "Gate in-place staged row ($stamp): Alex walked the pier at dawn." && ok=$((ok + 1))
  tk_add "$ENGINE_MEM" "baseline engine" core "Gate in-place core row ($stamp): Alex's middle name is Harbor." && ok=$((ok + 1))
  [ "$ok" = 5 ]
}

scenario_old() {
  local before after notes_before
  # v1 written in the config: stays v1 with rows in the store.
  write_config $'[memory]\nlinggen = "v1"\noutside_agents = "v1"'
  engine_up v1
  check "notes (old home, v1): the engine starts" $? "no health on $EP — $(tail -3 "$OUT/engine-v1.log")" "port $EP"
  if [ ! -d "$HOME/.cache/huggingface/hub" ]; then
    gap "notes (old home): v1 rows written through the engine" "no embedding model in $REAL_HOME/.cache/huggingface — the seeded half did not run"
    notes_stays_v1 "explicit v1 stays v1 (scratch engine)" "$WORK"
    return
  fi
  seed_v1_rows
  check "notes (old home): 5 v1 rows written through the engine (3 long-term, 1 staged, 1 core)" $? "$(tail -3 "$OUT/engine-v1.log" | head -c 300)" "$(wc -l <"$TK_ALL" | tr -d ' ') ids in the store"
  engine_down; engine_up v1-again
  check "notes (old home, v1): restarted on v1 with its rows" $? "no health after a restart" "port $EP"
  notes_stays_v1 "explicit v1 stays v1 (scratch engine)" "$WORK"
  engine_down

  # The default: the section taken out, as an upgrade from 2.6.1 leaves it.
  write_config ""
  engine_up default
  check "notes (old home, default): the engine starts on the old home" $? "no health on $EP — $(tail -3 "$OUT/engine-default.log")" "port $EP"
  notes_stays_v1 "upgrade on the default stays v1 (scratch engine)" "$WORK"
  engine_down

  # Written as v2 (opt-in): the old rows move into notes once.
  write_config $'[memory]\nlinggen = "v2"\noutside_agents = "v2"'
  engine_up v2
  check "notes (old home, v2): the engine starts on the old home" $? "no health on $EP — $(tail -3 "$OUT/engine-v2.log")" "port $EP"
  notes_moved "explicit v2 on a v1 home (scratch engine)" "$TK_IDS"
  notes_door "explicit v2 on a v1 home (scratch engine)"
  notes_prompts "explicit v2 on a v1 home (scratch engine)" yes "$WORK"
  # Once: a second start looks at the record and moves nothing again.
  before="$(cat "$NOTES_MOVED" 2>/dev/null)"; notes_before="$(notes_files | xargs -I{} shasum "$NOTES/{}" 2>/dev/null | shasum)"
  engine_down; engine_up v2-again
  after="$(cat "$NOTES_MOVED" 2>/dev/null)"
  [ -n "$before" ] && [ "$before" = "$after" ] && [ "$notes_before" = "$(notes_files | xargs -I{} shasum "$NOTES/{}" 2>/dev/null | shasum)" ]
  check "explicit v2 on a v1 home (scratch engine): a second start moves nothing again" $? "record or notes changed on the second start" "record and every note file as before"
  engine_down
}

run_scenario fresh
run_scenario old
echo "notes-local: homes under $S" >&2
