#!/usr/bin/env bash
# Live regression suite for the engine, against the RUNNING engine on 9527.
#
#   ./scripts/live-check.sh                core cases + model-calling cases
#   ./scripts/live-check.sh --no-model     core cases only (no model calls;
#                                          same as LIVE_SKIP_MODEL=1)
#   ./scripts/check.sh live [flags]        the same, from the check entry point
#
# Flags: --url URL (engine; default http://127.0.0.1:9527, env LIVE_URL),
#        --mem URL (the engine's ling-mem, read only; default
#          http://127.0.0.1:9528, env LIVE_MEM_URL),
#        --home DIR (the engine's state dir; default ~/.linggen, env LIVE_LINGGEN_HOME),
#        --alt-model ID (Yinyue's model in the two-models case; default
#          deepseek-flash, env LIVE_ALT_MODEL),
#        --speak (also run the app-moment case — she SPEAKS one line aloud to
#          any connected presenter; env LIVE_SPEAK=1),
#        --no-presence (don't beat presence during model runs; see below),
#        --keep (leave the scratch sessions, skill copy and folder).
#
# What it never touches: Lingjing's real save (data/state.json — its md5 is
# checked before and after), Yinyue's real daily thread (read and exported,
# never posted into), the real memory store (every scratch session that runs a
# model withholds the memory server — session `withheld_tools: [mcp__memory]`
# — so no member can write it; after the run, no row may name a scratch
# session as its source), any session it did not create. Everything scratch — the
# sessions (by exact id), the Lingjing copy under skills/, the chat folder
# under target/live-check — is removed on exit, failure included.
#
# Presence: a finished run or an open question while the user is "away" wakes
# Yinyue to herald it into her real daily thread (resident/triggers.rs,
# user_sees_screen). During model cases this script beats /api/presence every
# 3s so the engine reads the user as at the screen; the side effect is that
# real heralds are held back for the length of the run.
#
# Each case prints PASS/FAIL/SKIP with a short reason; exit status is non-zero
# on any FAIL.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
API="${LIVE_URL:-http://127.0.0.1:9527}"
MEM="${LIVE_MEM_URL:-http://127.0.0.1:9528}"
LG="${LIVE_LINGGEN_HOME:-$HOME/.linggen}"
ALT_MODEL="${LIVE_ALT_MODEL:-deepseek-flash}"
SOURCE_SKILL="${LIVE_SOURCE_SKILL:-lingjing}"
MODEL=1
[ "${LIVE_SKIP_MODEL:-0}" = 1 ] && MODEL=0
SPEAK="${LIVE_SPEAK:-0}"
PRESENCE=1
KEEP=0
RUN_TIMEOUT="${LIVE_RUN_TIMEOUT:-300}"
while [ $# -gt 0 ]; do
  case "$1" in
    --url) API="$2"; shift ;;
    --mem) MEM="$2"; shift ;;
    --home) LG="$2"; shift ;;
    --alt-model) ALT_MODEL="$2"; shift ;;
    --no-model) MODEL=0 ;;
    --speak) SPEAK=1 ;;
    --no-presence) PRESENCE=0 ;;
    --keep) KEEP=1 ;;
    -h|--help) sed -n '2,37p' "$0"; exit 0 ;;
    *) echo "live-check: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done

for tool in jq curl python3 rsync; do
  command -v "$tool" >/dev/null || { echo "live-check: needs $tool" >&2; exit 2; }
done
python3 -c 'import yaml' 2>/dev/null || { echo "live-check: needs python3 with PyYAML" >&2; exit 2; }

WORK="$REPO/target/live-check"
mkdir -p "$WORK"
TAG="lc$(date +%s)$$"
SKILL_SRC="$LG/skills/$SOURCE_SKILL"
SCRATCH_SKILL="lc-$SOURCE_SKILL-$TAG"
SCRATCH_SKILL_DIR="$LG/skills/$SCRATCH_SKILL"
# Not an OS temp dir: those are always-allowed (permission/model.rs step 3b),
# and the permission case needs a folder that asks.
CHAT_DIR="$WORK/chat-$TAG"
REAL_STATE="$SKILL_SRC/data/state.json"
TODAY="$(date +%F)"
YINYUE_SID="sess-yinyue-$TODAY"

# ── Reporting ───────────────────────────────────────────────────────────────

N_PASS=0 N_FAIL=0 N_SKIP=0
RESULTS="$WORK/results.txt"
: >"$RESULTS"
report() { # status name reason
  printf '%-4s  %-58s %s\n' "$1" "$2" "${3:-}" | tee -a "$RESULTS"
  case "$1" in PASS) N_PASS=$((N_PASS + 1)) ;; FAIL) N_FAIL=$((N_FAIL + 1)) ;; SKIP) N_SKIP=$((N_SKIP + 1)) ;; esac
}
pass() { report PASS "$1" "${2:-}"; }
fail() { report FAIL "$1" "${2:-}"; }
skip() { report SKIP "$1" "${2:-}"; }
section() { printf '\n── %s\n' "$1"; }
# check NAME CONDITION-EXIT FAIL-REASON [PASS-NOTE] — CONDITION-EXIT is $?.
check() { if [ "$2" = 0 ]; then pass "$1" "${4:-}"; else fail "$1" "$3"; fi; }

# expect NAME JSON JQ-PREDICATE FAIL-REASON [PASS-NOTE]
expect() {
  if printf '%s' "$2" | jq -e "$3" >/dev/null 2>&1; then pass "$1" "${5:-}"
  else fail "$1" "$4 — got: $(printf '%s' "$2" | head -c 240 | tr '\n' ' ')"; fi
}

# ── Scratch state and cleanup ───────────────────────────────────────────────

# Session ids are written to a file: new_session runs in $(…), a subshell,
# where an array append would be lost.
SESSIONS_FILE="$WORK/sessions-$TAG"
: >"$SESSIONS_FILE"
BEAT_PID=""
STATE_MD5_BEFORE="$(md5 -q "$REAL_STATE" 2>/dev/null || echo none)"
YINYUE_ROWS_BEFORE="$(wc -l <"$LG/sessions/$YINYUE_SID/messages.jsonl" 2>/dev/null | tr -d ' ' || echo 0)"
[ -n "$YINYUE_ROWS_BEFORE" ] || YINYUE_ROWS_BEFORE=0

cleanup() {
  [ -n "$BEAT_PID" ] && kill "$BEAT_PID" 2>/dev/null
  [ "$KEEP" = 1 ] && { echo "live-check: --keep: sessions $(tr '\n' ' ' <"$SESSIONS_FILE") / $SCRATCH_SKILL_DIR / $CHAT_DIR left"; return; }
  # Any question still open in a scratch session: dismissed, so no run waits on it.
  local sid
  while read -r sid <&3; do
    [ -n "$sid" ] || continue
    dismiss_asks "$sid" "" >/dev/null 2>&1
    curl -s -m10 -X DELETE -H 'content-type: application/json' \
      -d "$(jq -nc --arg s "$sid" '{session_id:$s}')" "$API/api/sessions/all" >/dev/null 2>&1
    case "$sid" in sess-*) [ -d "$LG/sessions/$sid" ] && rm -rf "${LG:?}/sessions/$sid" ;; esac
  done 3<"$SESSIONS_FILE"
  rm -f "$SESSIONS_FILE"
  case "$SCRATCH_SKILL_DIR" in
    "$LG"/skills/lc-*) if [ -d "$SCRATCH_SKILL_DIR" ]; then
        rm -rf "$SCRATCH_SKILL_DIR"
        curl -s -m20 -X POST -H 'content-type: application/json' -d '{}' "$API/api/skills/reload" >/dev/null 2>&1
      fi ;;
  esac
  case "$CHAT_DIR" in */target/live-check/chat-*) rm -rf "$CHAT_DIR" ;; esac
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# ── Engine calls ────────────────────────────────────────────────────────────

get() { curl -sS -m60 "$API$1" 2>&1; }
post() { curl -sS -m60 -X POST -H 'content-type: application/json' -d "$2" "$API$1" 2>&1; }
# A scratch session that runs a model: no member may write the real memory
# store (SessionMeta::withheld_tools — the whole memory server).
NO_MEMORY='{"withheld_tools":["mcp__memory"]}'
new_session() { # JSON → id (registered for cleanup)
  local id
  id="$(post /api/sessions "$1" | jq -r '.id // empty' 2>/dev/null)"
  [ -n "$id" ] && printf '%s\n' "$id" >>"$SESSIONS_FILE"
  printf '%s' "$id"
}
export_prompt() { # sid agent [project_root]
  get "/api/chat/system-prompt?project_root=$(jq -rn --arg r "${3:-}" '$r|@uri')&agent_id=$2&session_id=$1"
}
tool_names() { jq -c '[.tools[]? | (.function.name // .name)]' 2>/dev/null; }
chat() { # sid root message → the reply ({status, agent_id})
  post /api/chat "$(jq -nc --arg s "$1" --arg r "$2" --arg m "$3" '{project_root:$r, session_id:$s, message:$m}')"
}
rows_file() { printf '%s/sessions/%s/messages.jsonl' "$LG" "$1"; }
row_count() { wc -l <"$(rows_file "$1")" 2>/dev/null | tr -d ' ' || echo 0; }
meta_agents() { # sid → [ids] from session.yaml
  python3 - "$LG/sessions/$1/session.yaml" <<'PY' 2>/dev/null
import sys, yaml, json
m = yaml.safe_load(open(sys.argv[1])) or {}
print(json.dumps([a.get("id") for a in (m.get("agents") or [])]))
PY
}

# Pending questions for a session (optionally one agent's): one compact JSON per line.
pending_for() { # sid [agent]
  get /api/pending-ask-user | jq -c --arg s "$1" --arg a "${2:-}" \
    '.[]? | select(.session_id == $s and ($a == "" or .agent_id == $a))' 2>/dev/null
}
# Dismiss open questions without an answer (an empty answer = skipped).
dismiss_asks() { # sid [agent]
  local q
  pending_for "$1" "${2:-}" | while read -r q; do
    post /api/ask-user-response "$(printf '%s' "$q" | jq -c '{question_id, answers: []}')" >/dev/null
  done
}

# ── The log ─────────────────────────────────────────────────────────────────

log_file() { printf '%s/logs/linggen.%s' "$LG" "$(date -u +%F)"; }
log_mark() { local f; f="$(log_file)"; printf '%s:%s' "$f" "$(wc -c <"$f" 2>/dev/null | tr -d ' ' || echo 0)"; }
log_since() { # MARK → the log text written after it, colours stripped
  local f="${1%:*}" off="${1##*:}" now
  now="$(log_file)"
  { tail -c "+$((off + 1))" "$f" 2>/dev/null
    [ "$now" != "$f" ] && cat "$now" 2>/dev/null; } | sed $'s/\x1b\\[[0-9;]*m//g'
}
# run ids that began in SID since MARK, for AGENT.
runs_since() { # mark sid agent
  log_since "$1" | grep -E "run/begin .*agent_id=\"?$3\"? session_id=\"?$2\"?( |$)" \
    | sed -E 's/.*run\/begin run_id=([^ ]+).*/\1/' | sort -u
}

# wait_run SID AGENT MARK [TIMEOUT] — 0 finished, 2 a question of AGENT's is
# open in SID (its JSON in $ASK), 1 timeout.
ASK=""
wait_run() {
  local sid="$1" agent="$2" mark="$3" t="${4:-$RUN_TIMEOUT}" start
  start=$(date +%s)
  ASK=""
  sleep 2
  while :; do
    # (never `… | grep -q` under pipefail: grep's early exit SIGPIPEs the writer)
    if grep -qE "run/finish .*agent_id=\"?$agent\"? session_id=\"?$sid\"?( |$)" <<<"$(log_since "$mark")"; then
      return 0
    fi
    ASK="$(pending_for "$sid" "$agent" | head -1)"
    [ -n "$ASK" ] && return 2
    [ $(( $(date +%s) - start )) -gt "$t" ] && return 1
    sleep 3
  done
}
# A Ling turn of a closing-ask skill parks on the skill's question: skip it.
finish_ling() { # sid mark
  local rc
  wait_run "$1" ling "$2"; rc=$?
  while [ "$rc" = 2 ]; do
    dismiss_asks "$1" ling
    wait_run "$1" ling "$2" 120; rc=$?
  done
  return "$rc"
}

# ── Row analysis (python: the rows are JSON lines with free text inside) ────

rows_py() { # cmd sid from [args…]
  python3 - "$@" "$LG" <<'PY'
import json, sys, os
cmd, sid, start, *args = sys.argv[1:]
lg = args.pop()
rows = []
try:
    with open(os.path.join(lg, "sessions", sid, "messages.jsonl")) as f:
        rows = [json.loads(l) for l in f if l.strip()]
except FileNotFoundError:
    pass
new = rows[int(start):]

def is_call(r, agent):
    if not r.get("is_observation") or r.get("from_id") != agent:
        return None
    try:
        j = json.loads(r["content"])
    except Exception:
        return None
    return j.get("tool") if isinstance(j, dict) and j.get("type") == "tool" else None

if cmd == "calls":            # agent → tool names it called
    print(json.dumps([t for r in new if (t := is_call(r, args[0]))]))
elif cmd == "last_said":      # agent → its last spoken line
    said = [r["content"] for r in new if not r.get("is_observation") and r.get("from_id") == args[0]]
    print(said[-1] if said else "")
elif cmd == "look_place":     # place and people names from Ling's latest Look result
    names = []
    for r in new:
        c = r.get("content", "")
        if r.get("is_observation") and c.startswith("Tool Look:") and "STDOUT:" in c:
            body = c.split("STDOUT:", 1)[1].split("\nSTDERR", 1)[0].strip()
            try:
                j = json.loads(body)
            except Exception:
                continue
            scene = j.get("scene") or {}
            ps = [(j.get("place") or {}).get("name") or "", scene.get("place") or ""]
            names = {p.strip() for s in ps for p in s.split("·") if len(p.strip()) >= 2}
            names |= {p.get("name", "") for p in scene.get("people") or [] if len(p.get("name", "")) >= 2}
            # She reads what Look declares for others (`others_read`: the
            # player's `name`, the place, the scene); an engine before that
            # read the head of the output, where only `name` sat.
            if len(j.get("name") or "") >= 2:
                names.add(j["name"])
            names = sorted(names)
    print(json.dumps(names, ensure_ascii=False))
elif cmd == "compactions":    # count of compaction rows, and the index of the newest
    idx = [i for i, r in enumerate(rows) if not r.get("is_observation") and r.get("from_id") == "compaction"]
    print(json.dumps({"count": len(idx), "last": idx[-1] if idx else -1, "total": len(rows)}))
elif cmd == "hidden":         # [HIDDEN] rows since start: who holds them
    print(json.dumps([{"agent": r.get("agent_id"), "from": r.get("from_id")} for r in new if "[HIDDEN]" in r.get("content", "")]))
elif cmd == "grep":           # rows since start whose content holds args[0]
    print(sum(1 for r in new if args[0] in r.get("content", "")))
PY
}

# ── Presence: keep the user "at the screen" during model runs ───────────────

start_beat() {
  [ "$PRESENCE" = 1 ] || return 0
  ( while :; do
      curl -s -m3 -X POST -H 'content-type: application/json' \
        -d '{"focused":true,"typing":false,"idle_ms":0}' "$API/api/presence" >/dev/null 2>&1
      sleep 3
    done ) &
  BEAT_PID=$!
}

# ═══════════════════════════════════════════════════════════════════════════
# 1. Core cases — no model calls
# ═══════════════════════════════════════════════════════════════════════════

section "Engine ($API, state $LG)"
H="$(get /api/health)"
expect "health" "$H" '.ok == true' "unhealthy or unreachable" "$(printf '%s' "$H" | jq -r '.version // empty' 2>/dev/null)"
printf '%s' "$H" | jq -e '.ok == true' >/dev/null 2>&1 || { printf '\n%d passed, %d failed, %d skipped (engine down)\n' "$N_PASS" "$N_FAIL" "$N_SKIP"; exit 1; }

# The process on the port, and the state dir it resolves (paths.rs: $LINGGEN_HOME, else ~/.linggen).
PORT="${API##*:}"; PORT="${PORT%%/*}"
PID="$(lsof -t -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null | head -1)"
if [ -n "$PID" ]; then
  ENV_LH="$(ps eww -p "$PID" 2>/dev/null | tr ' ' '\n' | sed -n 's/^LINGGEN_HOME=//p' | head -1)"
  ENV_HOME="$(ps eww -p "$PID" 2>/dev/null | tr ' ' '\n' | sed -n 's/^HOME=//p' | head -1)"
  RESOLVED="${ENV_LH:-${ENV_HOME:-?}/.linggen}"
  [ "$RESOLVED" = "$LG" ]
  check "engine resolves its state dir to $LG" $? "process $PID resolves $RESOLVED (LINGGEN_HOME=${ENV_LH:-unset}, HOME=${ENV_HOME:-?})" "pid $PID, LINGGEN_HOME ${ENV_LH:-unset}"
else
  skip "engine resolves its state dir to $LG" "no LISTEN process found on port $PORT (remote engine?)"
fi

section "Sessions"
SID_C="$(new_session "$(jq -nc --arg r "$HOME" '{title:"live-check core", project_root:$r}')")"
[ -n "$SID_C" ] && [ -f "$LG/sessions/$SID_C/session.yaml" ]
check "session create lands in \$LINGGEN_HOME/sessions" $? "id='$SID_C', no $LG/sessions/$SID_C/session.yaml" "$SID_C"
SID_D="$(new_session '{"title":"live-check delete"}')"
curl -s -m10 -X DELETE -H 'content-type: application/json' -d "$(jq -nc --arg s "$SID_D" '{session_id:$s}')" "$API/api/sessions/all" >/dev/null
[ -n "$SID_D" ] && [ ! -e "$LG/sessions/$SID_D" ]
check "session delete removes it" $? "$LG/sessions/$SID_D still there"

section "Memory in the prompt"
# The engine's session_start call to ling-mem has a 2s cap; a cold daemon can
# miss it once. Warm it, and give an export a retry when core did not load:
# an older engine renders that as the empty-core text, a newer one (6531552,
# prompt/core_block.rs) as a timeout note. Either is a retry, never a pass —
# the checks below still demand the core heading.
WARM="$(export_prompt "$SID_C" ling)"
CORE_MISSED='has no `tier=core` rows yet|Core memory did not load in time for this turn'
memory_export() { # sid agent root → the export, retried once if core did not load
  local e
  e="$(export_prompt "$@")"
  if grep -qE "$CORE_MISSED" <<<"$(printf '%s' "$e" | jq -r '.system_prompt // empty')"; then
    sleep 2; e="$(export_prompt "$@")"
  fi
  printf '%s' "$e"
}
E="$(memory_export "$SID_C" ling)"
SP="$(printf '%s' "$E" | jq -r '.system_prompt // empty')"
grep -q '^## Core memory — who the user is' <<<"$SP" \
  && ! grep -q '^Memory scopes here: ' <<<"$SP" && ! grep -q '^## Index — ' <<<"$SP"
check "\$HOME session: core rows only (no candidates, no index)" $? \
  "core=$(grep -c '^## Core memory' <<<"$SP") candidates=$(grep -c '^Memory scopes here: ' <<<"$SP") index=$(grep -c '^## Index — ' <<<"$SP")"
SID_R="$(new_session "$(jq -nc --arg r "$REPO" '{title:"live-check repo", project_root:$r}')")"
E="$(memory_export "$SID_R" ling)"
SP="$(printf '%s' "$E" | jq -r '.system_prompt // empty')"
grep -q '^Memory scopes here: ' <<<"$SP"
check "project session: the candidates line" $? "no '^Memory scopes here: ' line in the export at $REPO" "$(grep -m1 '^Memory scopes here: ' <<<"$SP" | cut -c1-70)…"
grep -q '^## Index — ' <<<"$SP"
check "project session: the index" $? "no '^## Index — ' heading in the export at $REPO" "$(grep -c '^## Index — ' <<<"$SP") heading(s)"

section "System-prompt export per member"
for who in ling yinyue; do
  E="$(export_prompt "$SID_R" "$who")"
  expect "export $who: {system_prompt, tools, provider, model_id}" "$E" \
    '(.system_prompt | length) > 1000 and (.tools | length) > 0 and (.provider | length) > 0 and (.model_id | length) > 0' \
    "export incomplete" "$(printf '%s' "$E" | jq -r '"\(.provider)/\(.model_id), \(.tools | length) tools"' 2>/dev/null)"
done

section "Scratch sessions withhold the memory server"
mem_tools() { jq -c '[.[] | select(startswith("mcp__memory__"))]' 2>/dev/null; }
SID_W="$(new_session "$(jq -nc --arg r "$HOME" --argjson w "$NO_MEMORY" '{title:"live-check withheld", project_root:$r} + $w')")"
if ! grep -q '^withheld_tools:' "$LG/sessions/$SID_W/session.yaml" 2>/dev/null; then
  skip "scratch session: no member is offered a memory tool" "this engine predates session withheld_tools (redeploy) — the store check below still guards"
else
  CONTROL="$(export_prompt "$SID_C" ling | tool_names | mem_tools)"
  for who in ling yinyue; do
    MT="$(export_prompt "$SID_W" "$who" | tool_names | mem_tools)"
    expect "scratch session: $who is offered no memory tool" "$MT" 'length == 0' \
      "offered $MT" "an ordinary session offers $(printf '%s' "$CONTROL" | jq 'length' 2>/dev/null || echo ?)"
  done
fi

section "Yinyue's daily thread ($YINYUE_SID, read-only)"
if [ -f "$LG/sessions/$YINYUE_SID/session.yaml" ]; then
  AG="$(meta_agents "$YINYUE_SID")"
  expect "daily thread meta: agents [{id: yinyue}]" "$AG" '. == ["yinyue"]' "agents differ"
  PET_MODEL="$(get /api/config | jq -r '.pet.model // empty')"
  E="$(export_prompt "$YINYUE_SID" yinyue)"
  printf '%s' "$E" | jq -e --arg m "$PET_MODEL" '.model_id == $m' >/dev/null 2>&1
  check "daily thread export: her model is pet.model" $? "model_id=$(printf '%s' "$E" | jq -r .model_id) pet.model=$PET_MODEL" "$PET_MODEL"
  TN="$(printf '%s' "$E" | tool_names)"
  expect "daily thread export: her narrow tools (no Bash/Write/Skill)" "$TN" \
    'length > 0 and (index("Bash") == null) and (index("Write") == null) and (index("Skill") == null) and (index("Edit") == null)' \
    "a worker tool in her set" "$(printf '%s' "$TN" | jq 'length') tools"
else
  skip "daily thread meta / export" "no $YINYUE_SID yet today"
fi

section "Scratch $SOURCE_SKILL copy ($SCRATCH_SKILL)"
make_skill_copy() {
  [ -d "$SKILL_SRC" ] || return 1
  rsync -a --exclude 'data/saves' --exclude 'data/log.jsonl' --exclude 'data/state.json*' \
    "$SKILL_SRC/" "$SCRATCH_SKILL_DIR/" || return 1
  # Her place reads companion.awake: the copy has her awake beside the player.
  jq '.companion = {joined: (.companion.joined // "2026-01-01"), awake: true}' "$REAL_STATE" \
    >"$SCRATCH_SKILL_DIR/data/state.json" || return 1
  python3 - "$SCRATCH_SKILL_DIR/SKILL.md" "$SOURCE_SKILL" "$SCRATCH_SKILL" <<'PY' || return 1
import sys, re
path, old, new = sys.argv[1:]
text = open(path).read()
head, sep, body = text[4:].partition("\n---")
out, drop = [], False
for line in head.split("\n"):
    top = re.match(r"^([A-Za-z][\w-]*):", line)
    if top:
        drop = top.group(1) in ("app", "cloud")
    if drop:
        continue
    if line.startswith("name:"):
        line = f"name: {new}"
    elif line.startswith("cwd:"):
        line = f"cwd: ~/.linggen/skills/{new}"
    line = line.replace(f"~/.linggen/skills/{old},", f"~/.linggen/skills/{new},")
    out.append(line)
open(path, "w").write("---\n" + "\n".join(out) + sep + body)
PY
  post /api/skills/reload '{}' >/dev/null
}
SID_A=""
if make_skill_copy; then
  pass "scratch skill copy + reload" "$SCRATCH_SKILL_DIR"
  SID_A="$(new_session "$(jq -nc --arg k "$SCRATCH_SKILL" --argjson w "$NO_MEMORY" '{title:"live-check table", skill:$k} + $w')")"
  AG="$(meta_agents "$SID_A")"
  expect "skill session: members ling + yinyue" "$AG" 'index("ling") != null and index("yinyue") != null' "members differ"
  E="$(export_prompt "$SID_A" ling)"; TN="$(printf '%s' "$E" | tool_names)"
  expect "export at the skill: Ling lists the skill's own tools (Look, Resolve)" "$TN" \
    'index("Look") != null and index("Resolve") != null' \
    "Ling's export lacks the skill's tools (regression of cecc2f4 'Export lists the skill's own tools')" \
    "$(printf '%s' "$TN" | jq 'length') tools"
  E="$(export_prompt "$SID_A" yinyue)"; TN="$(printf '%s' "$E" | tool_names)"
  expect "export at the skill: Yinyue lists her place's tools (AppTool, Story)" "$TN" \
    'index("AppTool") != null and index("Story") != null and (index("Resolve") == null)' \
    "her export lacks Story/AppTool or holds Resolve (regression of cecc2f4)" \
    "$(printf '%s' "$TN" | jq -c '.')"
else
  fail "scratch skill copy + reload" "could not copy $SKILL_SRC"
fi

# ═══════════════════════════════════════════════════════════════════════════
# 2. Model-calling cases (--no-model / LIVE_SKIP_MODEL=1 skips them)
# ═══════════════════════════════════════════════════════════════════════════

MODEL_CASES=("(a) shared table: Ling Looks, @银月 answers from it" "(b) @银月 joins a chat; her Write asks; Deny writes nothing" \
  "(d) forced shared compaction" "(e) two members on two models")
if [ "$MODEL" = 0 ]; then
  section "Model cases"
  for c in "${MODEL_CASES[@]}" "(f) app moment at a table"; do skip "$c" "--no-model"; done
else
  start_beat
  NOTE="（自动测试，请勿写入记忆。）"

  # ── (a) ──────────────────────────────────────────────────────────────────
  section "${MODEL_CASES[0]}"
  if [ -z "$SID_A" ]; then
    skip "(a) shared table" "no scratch skill session"
  else
    R0="$(row_count "$SID_A")"; M="$(log_mark)"
    R="$(chat "$SID_A" "" "（测试）请先调用 Look 看看现在的情形，然后只回复「好」一个字，别的都不要说。${NOTE}")"
    expect "(a) plain message → ling" "$R" '.agent_id == "ling"' "routed elsewhere"
    finish_ling "$SID_A" "$M"; check "(a) Ling's turn finishes" $? "no run/finish for ling in $SID_A"
    CALLS="$(rows_py calls "$SID_A" "$R0" ling)"
    expect "(a) Ling called Look" "$CALLS" 'index("Look") != null' "no Look call"
    PLACE="$(rows_py look_place "$SID_A" "$R0")"
    R1="$(row_count "$SID_A")"; M="$(log_mark)"
    R="$(chat "$SID_A" "" "@银月 照 Ling 刚才 Look 的结果：玩家叫什么名字？只说名字。${NOTE}")"
    expect "(a) @银月 → yinyue" "$R" '.agent_id == "yinyue"' "routed elsewhere"
    wait_run "$SID_A" yinyue "$M"; RC=$?
    [ "$RC" = 2 ] && dismiss_asks "$SID_A" yinyue
    [ "$RC" = 0 ]; check "(a) her turn finishes with no AskUser" $? "rc=$RC ${ASK:0:200}"
    CALLS="$(rows_py calls "$SID_A" "$R1" yinyue)"
    expect "(a) her turn makes no tool calls" "$CALLS" 'length == 0' "she called tools"
    SAID="$(rows_py last_said "$SID_A" "$R1" yinyue)"
    printf '%s' "$PLACE" | jq -e --arg s "$SAID" 'length > 0 and any(.[]; . as $p | $s | contains($p))' >/dev/null 2>&1
    check "(a) her answer uses Ling's Look result" $? "Look named $PLACE; she said: ${SAID:0:160}" "${SAID:0:60}"
    dismiss_asks "$SID_A" ling
    R2="$(row_count "$SID_A")"; M="$(log_mark)"
    R="$(chat "$SID_A" "" "（测试）好的。只回复「嗯」。${NOTE}")"
    expect "(a) next plain message → ling" "$R" '.agent_id == "ling"' "the table stayed with her"
    finish_ling "$SID_A" "$M" >/dev/null
  fi

  # ── (b) ──────────────────────────────────────────────────────────────────
  # Her model sometimes answers a write request with Glob/Read and no Write, so
  # no ask appears. The message names the exact tool and arguments; a turn with
  # no Write gets one firmer retry; still none → SKIP (nothing was proved). A
  # Write that ran without an ask, or a file after Deny, is always a FAIL.
  section "${MODEL_CASES[1]}"
  mkdir -p "$CHAT_DIR"
  SID_B="$(new_session "$(jq -nc --arg r "$CHAT_DIR" --argjson w "$NO_MEMORY" '{title:"live-check chat", project_root:$r} + $w')")"
  PROBE="$CHAT_DIR/lc-probe.txt"
  # Write calls by her runs in SID_B since MARK (the log line is written when
  # the call starts, before its permission check).
  her_writes() { # mark → count
    local txt rid n=0
    txt="$(log_since "$1")"
    for rid in $(runs_since "$1" "$SID_B" yinyue); do
      n=$((n + $(grep -cF "[$rid] Tool: Write " <<<"$txt")))
    done
    printf '%s' "$n"
  }
  deny_ask() { # $ASK → answered with its Deny option
    post /api/ask-user-response "$(jq -nc --arg q "$(printf '%s' "$ASK" | jq -r .question_id)" --arg d "$DENY" \
      '{question_id:$q, answers:[{question_index:0, selected:[$d], custom_text:null}]}')" >/dev/null
  }
  B_MSGS=("@银月 这是权限测试：请立刻调用 Write 工具，参数 path=\"lc-probe.txt\"，content=\"hello\"。只调用 Write 这一个工具，不要调用 Glob、Read 或别的工具，不要先问我。${NOTE}"
    "@银月 你上一轮没有调用 Write。现在第一步、也是唯一一步：调用 Write 工具，path 为 \"lc-probe.txt\"，content 为 \"hello\"。不要检查文件是否存在，不要调用其他工具，不要回复文字后再停下。${NOTE}")
  B_DONE=""   # asked | noask | timeout | nowrite
  for B_TRY in 0 1; do
    R0="$(row_count "$SID_B")"; M="$(log_mark)"
    R="$(chat "$SID_B" "$CHAT_DIR" "${B_MSGS[$B_TRY]}")"
    [ "$B_TRY" = 0 ] && expect "(b) @银月 → yinyue" "$R" '.agent_id == "yinyue"' "routed elsewhere"
    wait_run "$SID_B" yinyue "$M"; RC=$?
    sleep 1
    W="$(her_writes "$M")"
    if [ "$RC" = 2 ] && [ "$W" -gt 0 ]; then B_DONE=asked; break; fi
    # An ask with no Write before it is her own question, not a permission ask.
    [ "$RC" = 2 ] && { dismiss_asks "$SID_B" yinyue; wait_run "$SID_B" yinyue "$M" 120; RC=$?; W="$(her_writes "$M")"; }
    if [ "$W" -gt 0 ]; then B_DONE=noask; break; fi
    [ "$RC" = 1 ] && { B_DONE=timeout; break; }
    [ "$RC" = 2 ] && dismiss_asks "$SID_B" yinyue
    B_DONE=nowrite
  done
  case "$B_DONE" in
    asked)
      DENY="$(printf '%s' "$ASK" | jq -r '[.questions[0].options[]?.label | select(test("deny"; "i"))][0] // "Deny"')"
      pass "(b) her Write asks permission" "try $((B_TRY + 1)): $(printf '%s' "$ASK" | jq -r '.questions[0].question' | head -c 60)"
      deny_ask
      # She may try again after a Deny: each new prompt is denied too (at most 3).
      for _ in 1 2 3; do
        wait_run "$SID_B" yinyue "$M"; RC=$?
        [ "$RC" = 2 ] || break
        deny_ask
      done
      [ "$RC" = 2 ] && dismiss_asks "$SID_B" yinyue
      [ "$RC" = 0 ]; check "(b) her turn finishes after Deny" $? "rc=$RC (1 timeout, 2 still asking)" ;;
    noask)
      fail "(b) her Write asks permission" "Write ran with no permission prompt (try $((B_TRY + 1)), rc=$RC)" ;;
    timeout)
      fail "(b) her Write asks permission" "her turn timed out with no prompt (try $((B_TRY + 1)); calls $(rows_py calls "$SID_B" "$R0" yinyue))" ;;
    *)
      skip "(b) her Write asks permission" "her model called no Write in 2 tries (last calls $(rows_py calls "$SID_B" "$R0" yinyue)) — nothing to ask about" ;;
  esac
  [ ! -e "$PROBE" ]; check "(b) Deny → no file written" $? "$PROBE exists"
  expect "(b) she stays in session.yaml agents" "$(meta_agents "$SID_B")" 'index("yinyue") != null' "not a member"

  # ── (d) ──────────────────────────────────────────────────────────────────
  section "${MODEL_CASES[2]}"
  # The cut keeps a verbatim tail of 15% of the smallest member window (~19k
  # tokens at 128k): a ~30k-token filler line leaves something to summarize.
  FILLER="$(python3 -c 'import sys; print(" ".join(f"filler-{i:05d} the quick brown fox jumps over the lazy dog." for i in range(2600)))')"
  M="$(log_mark)"
  chat "$SID_B" "$CHAT_DIR" "（测试，以下是填充文本，请忽略，不要用任何工具，只回复 OK。${NOTE}）
$FILLER" >/dev/null
  wait_run "$SID_B" ling "$M"; RC=$?; [ "$RC" = 2 ] && dismiss_asks "$SID_B" ling
  M="$(log_mark)"
  chat "$SID_B" "$CHAT_DIR" "（测试）只回复「好」。${NOTE}" >/dev/null
  wait_run "$SID_B" ling "$M"; RC=$?; [ "$RC" = 2 ] && dismiss_asks "$SID_B" ling
  BEFORE="$(rows_py compactions "$SID_B" 0)"
  C="$(curl -sS -m300 -X POST -H 'content-type: application/json' -d "$(jq -nc --arg s "$SID_B" --arg r "$CHAT_DIR" '{project_root:$r, session_id:$s}')" "$API/api/chat/compact")"
  expect "(d) /api/chat/compact compacts" "$C" '.compacted == true' "not compacted"
  AFTER="$(rows_py compactions "$SID_B" 0)"
  expect "(d) exactly one compaction row" "$AFTER" ".count == 1" "before $BEFORE"

  # ── (e) is set up here: the turns after the cut run on two models ──────
  if get /api/models | jq -e --arg m "$ALT_MODEL" 'any(.[]?; .id == $m)' >/dev/null 2>&1; then
    # No API sets a member's model: session.yaml agents: is the record.
    python3 - "$LG/sessions/$SID_B/session.yaml" "$ALT_MODEL" <<'PY'
import sys, yaml
p, model = sys.argv[1:]
m = yaml.safe_load(open(p))
agents = m.get("agents") or []
for a in agents:
    if a.get("id") == "yinyue":
        a["model"] = model
if not any(a.get("id") == "yinyue" for a in agents):
    agents.append({"id": "yinyue", "model": model})
m["agents"] = agents
yaml.safe_dump(m, open(p, "w"), allow_unicode=True, sort_keys=False)
PY
    ALT=1
  else
    ALT=0
  fi
  LAST="$(printf '%s' "$AFTER" | jq .last)"
  ME="$(log_mark)"
  M="$(log_mark)"
  R="$(chat "$SID_B" "$CHAT_DIR" "（测试）只回复「在」。${NOTE}")"
  wait_run "$SID_B" ling "$M"; RC=$?; [ "$RC" = 2 ] && dismiss_asks "$SID_B" ling
  LING_RUN="$(runs_since "$M" "$SID_B" ling | tail -1)"
  NL="$(log_since "$M" | grep -E "Rebuilt ling's thread from the session: [0-9]+ messages" | tail -1 | sed -E 's/.*: ([0-9]+) messages.*/\1/')"
  M="$(log_mark)"
  R="$(chat "$SID_B" "$CHAT_DIR" "@银月 只回复「在」。${NOTE}")"
  wait_run "$SID_B" yinyue "$M"; RC=$?; [ "$RC" = 2 ] && dismiss_asks "$SID_B" yinyue
  YY_RUN="$(runs_since "$M" "$SID_B" yinyue | tail -1)"
  NY="$(log_since "$M" | grep -E "Rebuilt yinyue's thread from the session: [0-9]+ messages" | tail -1 | sed -E 's/.*: ([0-9]+) messages.*/\1/')"
  SINCE=$(( $(row_count "$SID_B") - LAST ))
  [ -n "$NL" ] && [ "$NL" -ge 1 ] && [ "$NL" -le "$SINCE" ]
  check "(d) Ling's rebuilt thread starts at the summary" $? "rebuilt='${NL:-none}' rows since the summary=$SINCE" "$NL messages (≤ $SINCE rows since it)"
  [ -n "$NY" ] && [ "$NY" -ge 1 ] && [ "$NY" -le "$SINCE" ]
  check "(d) Yinyue's rebuilt thread starts at the summary" $? "rebuilt='${NY:-none}' rows since the summary=$SINCE" "$NY messages"

  section "${MODEL_CASES[3]}"
  if [ "$ALT" = 1 ]; then
    WIN="$(log_since "$ME")"
    LM="$(printf '%s' "$WIN" | grep -F "[$LING_RUN] Agent loop: model_id=" | head -1 | sed -E 's/.*model_id=([^,]+).*/\1/')"
    YM="$(printf '%s' "$WIN" | grep -F "[$YY_RUN] Agent loop: model_id=" | head -1 | sed -E 's/.*model_id=([^,]+).*/\1/')"
    [ -n "$LM" ] && [ "$YM" = "$ALT_MODEL" ] && [ "$LM" != "$YM" ]
    check "(e) Ling and Yinyue ran on different models" $? "ling[$LING_RUN]=${LM:-?} yinyue[$YY_RUN]=${YM:-?} (want $ALT_MODEL)" "ling $LM, yinyue $YM"
    BAD="$(printf '%s' "$WIN" | grep -E ' (WARN|ERROR) ' | grep -E "\[($LING_RUN|$YY_RUN)\]|src/provider/|run_id=($LING_RUN|$YY_RUN) " | head -3)"
    [ -z "$BAD" ]; check "(e) no provider errors/WARN for those runs" $? "$(printf '%s' "$BAD" | cut -c1-200 | tr '\n' ' ')"
    AGJ="$(python3 -c 'import sys,yaml,json; print(json.dumps(yaml.safe_load(open(sys.argv[1]))["agents"]))' "$LG/sessions/$SID_B/session.yaml")"
    printf '%s' "$AGJ" | jq -e --arg m "$ALT_MODEL" 'any(.[]; .id == "yinyue" and .model == $m)' >/dev/null 2>&1
    check "(e) her model kept in session.yaml" $? "$AGJ"
  else
    skip "(e) two members on two models" "$ALT_MODEL is not a model on this engine (--alt-model)"
  fi

  # ── (f) ──────────────────────────────────────────────────────────────────
  section "(f) app moment at a table"
  if [ "$SPEAK" != 1 ]; then
    skip "(f) app moment at a table" "speaks one line aloud — opt in with --speak"
  elif [ -z "$SID_A" ]; then
    skip "(f) app moment at a table" "no scratch skill session"
  else
    R0="$(row_count "$SID_A")"; M="$(log_mark)"
    R="$(post /api/yinyue/event "$(jq -nc --arg a "$SCRATCH_SKILL" --arg s "$SID_A" '{app:$a, text:"（测试）玩家抬头看了看天色。", asked:true, session:$s}')")"
    wait_run "$SID_A" yinyue "$M" 180; RC=$?
    [ "$RC" = 2 ] && dismiss_asks "$SID_A" yinyue
    [ "$RC" = 0 ]; check "(f) her moment turn runs at the table" $? "rc=$RC event→ ${R:0:80}"
    sleep 3
    SAID="$(rows_py last_said "$SID_A" "$R0" yinyue)"
    [ -n "$SAID" ]; check "(f) her line lands in the session" $? "no yinyue row" "${SAID:0:60}"
    HID="$(rows_py hidden "$SID_A" "$R0")"
    expect "(f) her hidden kickoff is hers alone (thread.rs: not in Ling's thread)" "$HID" \
      'length >= 1 and all(.[]; .agent == "yinyue")' "a [HIDDEN] row held by another member"
  fi

  [ -n "$BEAT_PID" ] && { kill "$BEAT_PID" 2>/dev/null; BEAT_PID=""; }

  section "Side effects"
  NEWY="$(python3 - "$LG/sessions/$YINYUE_SID/messages.jsonl" "$YINYUE_ROWS_BEFORE" <<'PY'
import sys, json
try:
    rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()][int(sys.argv[2]):]
except FileNotFoundError:
    rows = []
print(sum(1 for r in rows if "while the user was away" in r.get("content", "") or "is blocked, waiting on the user" in r.get("content", "")))
PY
)"
  [ "$NEWY" = 0 ]; check "no herald into her daily thread" $? "$NEWY herald kickoff(s) landed in $YINYUE_SID"

  # The real memory store, read only: no row may name a scratch session as
  # its source (the engine stamps source_session on every memory_add).
  if curl -s -m10 "$MEM/api/health" | jq -e '.ok == true' >/dev/null 2>&1; then
    WROTE=0 SEEN=""
    while read -r sid <&3; do
      [ -n "$sid" ] || continue
      N="$(curl -s -m60 -X POST -H 'content-type: application/json' \
        -d "$(jq -nc --arg s "$sid" '{source_session:$s, include_expired:true, limit:50}')" \
        "$MEM/api/memory/list" | jq '.data | length' 2>/dev/null)"
      [ -n "$N" ] || { WROTE=-1; SEEN="$SEEN $sid:unreadable"; continue; }
      [ "$N" = 0 ] || { WROTE=$((WROTE + N)); SEEN="$SEEN $sid:$N"; }
    done 3<"$SESSIONS_FILE"
    [ "$WROTE" = 0 ]; check "no memory row written by a scratch session" $? \
      "rows by source_session:$SEEN — remove them by id from $MEM" "$(wc -l <"$SESSIONS_FILE" | tr -d ' ') sessions checked"
  else
    skip "no memory row written by a scratch session" "ling-mem at $MEM unreachable (nor could a turn write it)"
  fi
fi

# The real save: never ours to write. A change is attributed, never restored.
STATE_MD5_AFTER="$(md5 -q "$REAL_STATE" 2>/dev/null || echo none)"
if [ "$STATE_MD5_BEFORE" = "$STATE_MD5_AFTER" ]; then
  pass "real $SOURCE_SKILL state.json untouched" "$STATE_MD5_AFTER"
else
  fail "real $SOURCE_SKILL state.json untouched" "md5 $STATE_MD5_BEFORE → $STATE_MD5_AFTER — check the log for a real $SOURCE_SKILL session's run in this window"
fi

# ── Summary ─────────────────────────────────────────────────────────────────

printf '\n%d passed, %d failed, %d skipped (engine %s at %s)\n' "$N_PASS" "$N_FAIL" "$N_SKIP" \
  "$(printf '%s' "$H" | jq -r '.version // "?"')" "$API"
[ "$N_FAIL" -eq 0 ]
