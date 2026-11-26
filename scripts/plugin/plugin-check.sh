#!/usr/bin/env bash
# Host-plugin checks for the linggen plugin (plugins/linggen): one version with
# the engine, install parity, the CC/Codex hooks, one address everywhere, and
# protocol-text alignment across every memory surface.
#
#   ./scripts/plugin/plugin-check.sh               version + parity + Codex
#                                                  schema + text grep
#   ./scripts/plugin/plugin-check.sh --sync        also copy drifted repo files
#                                                  over each installed copy
#   With --port/--home/--project/--skill the hooks also run for real against a
#   scratch Linggen (on --port) and a fixture home.
#
# Never writes the store; never `rsync --delete` into ~/.linggen/skills (that
# wipes skills' data/). --sync copies file by file and deletes nothing.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REAL_HOME="$HOME"
PORT="" FHOME="" PROJ="" SKILL="" SYNC=0
while [ $# -gt 0 ]; do
  case "$1" in
    --port) PORT="$2"; shift ;;
    --home) FHOME="$2"; shift ;;
    --project) PROJ="$2"; shift ;;
    --skill) SKILL="$2"; shift ;;
    --sync) SYNC=1 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "plugin-check: unknown flag $1" >&2; exit 2 ;;
  esac
  shift
done

N_FAIL=0
report() { printf '%-4s  %-52s %s\n' "$1" "$2" "${3:-}"; [ "$1" = FAIL ] && N_FAIL=$((N_FAIL + 1)); return 0; }
pass() { report PASS "$1" "${2:-}"; }
fail() { report FAIL "$1" "${2:-}"; }
skip() { report SKIP "$1" "${2:-}"; }
expect() {
  if printf '%s' "$2" | jq -e "$3" >/dev/null 2>&1; then pass "$1" "${5:-}"
  else fail "$1" "$4 — got: $(printf '%s' "$2" | head -c 200 | tr '\n' ' ')"; fi
}
md5of() { if command -v md5 >/dev/null; then md5 -q "$1"; else md5sum "$1" | cut -d' ' -f1; fi; }

# ── 1. Parity: repo plugin dirs vs every installed copy ─────────────────────

CCP="$REPO/plugins/linggen"
CC_VER="$(jq -r .version "$CCP/.claude-plugin/plugin.json")"

# ── 0. One version: the engine's, in every file the plugin carries it ───────

ENGINE_VER="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version = "\(.*\)"/\1/p;}' "$REPO/Cargo.toml" | head -1)"
V_FILE="$(tr -d '[:space:]' <"$CCP/VERSION")"
V_CODEX="$(jq -r .version "$CCP/.codex-plugin/plugin.json")"
if [ -n "$ENGINE_VER" ] && [ "$CC_VER" = "$ENGINE_VER" ] && [ "$V_CODEX" = "$ENGINE_VER" ] && [ "$V_FILE" = "v$ENGINE_VER" ]; then
  pass "version: plugin = engine" "$ENGINE_VER"
else
  fail "version: plugin = engine" "Cargo $ENGINE_VER · CC $CC_VER · Codex $V_CODEX · VERSION $V_FILE (scripts/sync-version.sh)"
fi

# parity NAME SRC DST [find-args…] — files SRC ships, compared by md5.
parity() {
  local name="$1" src="$2" dst="$3"; shift 3
  if [ ! -d "$dst" ]; then skip "parity: $name" "not installed ($dst)"; return; fi
  local drift=() missing=() f
  while IFS= read -r f; do
    if [ ! -f "$dst/$f" ]; then missing+=("$f")
    elif [ "$(md5of "$src/$f")" != "$(md5of "$dst/$f")" ]; then drift+=("$f"); fi
  done < <(cd "$src" && find . -type f ! -name .DS_Store "$@" | sed 's|^\./||' | sort)
  local bad=("${drift[@]+"${drift[@]}"}" "${missing[@]+"${missing[@]}"}")
  if [ ${#bad[@]} -eq 0 ]; then pass "parity: $name" "md5 match"; return; fi
  if [ "$SYNC" = 1 ]; then
    for f in "${bad[@]}"; do mkdir -p "$(dirname "$dst/$f")" && cp -p "$src/$f" "$dst/$f"; done
    pass "parity: $name" "synced ${#bad[@]}: ${bad[*]}"
  else
    fail "parity: $name" "${#drift[@]} drifted, ${#missing[@]} missing: ${bad[*]:0:6} (--sync)"
  fi
}
parity "CC marketplace" "$CCP" "$REAL_HOME/.claude/plugins/marketplaces/linggen/plugins/linggen"
parity "CC cache $CC_VER" "$CCP" "$REAL_HOME/.claude/plugins/cache/linggen/linggen/$CC_VER"
parity "Codex cache $CC_VER" "$CCP" "$REAL_HOME/.codex/plugins/cache/linggen/linggen/$CC_VER"
# ── 2. Codex hook declaration ───────────────────────────────────────────────

CX="$(cat "$CCP/hooks/codex.hooks.json")"
expect "Codex hooks: nested map keyed by event" "$CX" \
  '(.hooks | type == "object") and (.hooks | keys | all(IN("SessionStart","UserPromptSubmit","PreToolUse","PostToolUse","Stop"))) and ([.hooks[][] | .hooks[] | .type] | all(. == "command"))' \
  "not {hooks:{Event:[{hooks:[{type:command}]}]}}"
expect "Codex hooks: absolute \${CLAUDE_PLUGIN_ROOT} paths" "$CX" \
  '[.hooks[][] | .hooks[].command | sub("^([A-Z_]+=[^ ]+ )+"; "")] | all(startswith("${CLAUDE_PLUGIN_ROOT}/"))' "a relative command path"
for cmd in $(printf '%s' "$CX" | jq -r '.hooks[][] | .hooks[].command | sub("^([A-Z_]+=[^ ]+ )+"; "")' | sed 's|${CLAUDE_PLUGIN_ROOT}/||'); do
  [ -x "$CCP/$cmd" ] && pass "Codex hooks: $cmd executable" || fail "Codex hooks: $cmd executable" "missing or not +x"
done
CCH="$(cat "$CCP/hooks/hooks.json")"
for HJ in "$CCH" "$CX"; do
  expect "hooks: PreToolUse stamp matcher = add/search/upkeep" "$HJ" \
    '[.hooks.PreToolUse[] | select(.hooks[0].command | test("stamp-cwd"))][0].matcher as $m | ("mcp__plugin_linggen_linggen__memory_add" | test($m)) and ("mcp__linggen__memory_search" | test($m)) and ("mcp__x__memory_upkeep" | test($m)) and (("mcp__x__memory_delete" | test($m)) | not) and (("mcp__x__memory_update" | test($m)) | not)' \
    "matcher misses a tool, or takes one the hook never stamps"
done

# One address, one door: every declaration is the one `linggen` server at
# Linggen's /mcp. The retired port never appears.
MJ="$(cat "$CCP/.mcp.json")"
expect "CC .mcp.json: one linggen server at /mcp" "$MJ" \
  '(.mcpServers | keys) == ["linggen"] and (.mcpServers.linggen.url | endswith(":${LINGGEN_PORT:-9527}/mcp"))' \
  "servers or URL differ"
CXM="$(cat "$CCP/.codex-plugin/mcp.json")"
expect "Codex mcp.json: one linggen server at /mcp" "$CXM" \
  '(.mcpServers | keys) == ["linggen"] and .mcpServers.linggen.url == "http://127.0.0.1:9527/mcp"' \
  "servers or URL differ"
# The tool names the door retired, its old memory path and the old memory
# server's namespace: none is live in the plugin (a historical note may name
# them as such). The engine answers a retired name with "update the plugin".
# This list is the catalogue's retired names, checked against it by the
# door's repo_check (a name the catalogue retires and this misses fails it).
RETIRED_NAMES='memory_(list|get|session_start|harvest_day|days|remember_day|dream_lease|role|sweep|issues|issue_add|issue_resolve|chains|dream_status|dream_run)|agent_(stop|session_close|models)|browser_(navigate|tabs|read_page|screenshot|read_console|click|type|key|scroll|wait)|x_(search|targets|following|whotofollow|own|mentions)|plugin_linggen_memory'
# Bounded by hand, not -w: `_` is a word character, and a namespaced name
# (mcp__plugin_linggen_linggen__memory_dream_status) must still be seen.
gone="$(grep -rnIE "(__|[^A-Za-z0-9_]|^)($RETIRED_NAMES)([^A-Za-z0-9_]|$)" "$CCP" "$REPO/.claude-plugin" 2>/dev/null | grep -vE '/evals-isolated/results/|/upgrade-fixture/|[Hh]istorical' || true)
$(grep -rnIF '/mcp/memory' "$CCP" "$REPO/.claude-plugin" 2>/dev/null | grep -vE '/evals-isolated/results/|/upgrade-fixture/|[Hh]istorical|earlier versions' || true)"
gone="$(printf '%s' "$gone" | sed '/^$/d')"
[ -z "$gone" ] && pass "no retired tool names or /mcp/memory in the plugin" \
  || fail "no retired tool names or /mcp/memory in the plugin" "$(printf '%s' "$gone" | head -3 | tr '\n' ' ')"
old="$(grep -rnIE '9528|LING_MEM_(HOST|PORT|TOKEN)|ling-mem (start|restart|status|upgrade)|install-bin' "$CCP/.mcp.json" "$CCP/.codex-plugin" "$CCP/hooks" "$CCP/scripts" 2>/dev/null \
  | grep -vE 'retired|old memory|older file|legacy|_ling_pick|pick\(' || true)"
[ -z "$old" ] && pass "no retired address or daemon verbs in code" \
  || fail "no retired address or daemon verbs in code" "$(printf '%s' "$old" | head -3 | tr '\n' ' ')"

# The hooks' own offline tests (no Linggen needed). A hook asks the engine at
# LINGGEN_HOST:PORT whether memory is v2 (and then stays silent): a Linggen
# running on this machine must not answer, so the port is a closed one and the
# home has no client.json.
for t in "$CCP"/hooks/tests/*.test.sh; do
  name="hooks test: $(basename "$t" .test.sh)"
  if out="$(HOME=/nonexistent-home LINGGEN_HOST=127.0.0.1 LINGGEN_PORT=1 bash "$t" 2>&1)"; then pass "$name"
  else fail "$name" "$(printf '%s' "$out" | tail -3 | tr '\n' ' ')"; fi
done

# ── 3. Hooks against the scratch Linggen ────────────────────────────────────

if [ -n "$PORT" ] && [ -n "$FHOME" ] && [ -n "$PROJ" ]; then
  WORKP="$(dirname "$FHOME")/plugin"
  mkdir -p "$WORKP/bin"
  # A PATH with jq/curl/git/node only — no `ling`, so autostart cannot start
  # the real engine; the proxy answers health, so it starts nothing anyway.
  for t in jq curl git node python3 bash; do ln -sf "$(command -v "$t")" "$WORKP/bin/$t"; done

  # A recording proxy in front of the scratch engine: every MCP body a hook sends
  # lands in $LOG, so the checks can see the args, not just the answers.
  PPORT=$((PORT + 10))
  LOG="$WORKP/mcp.log"
  : >"$LOG"
  python3 - "$PPORT" "$PORT" "$LOG" <<'PY' &
import sys, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer
port, up, log = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        r = urllib.request.urlopen(f"http://127.0.0.1:{up}{self.path}")
        b = r.read(); self.send_response(r.status); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("content-length", 0)))
        with open(log, "ab") as f: f.write(body + b"\n")
        req = urllib.request.Request(f"http://127.0.0.1:{up}{self.path}", body, {"content-type": "application/json"})
        b = urllib.request.urlopen(req).read()
        self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(b)
HTTPServer(("127.0.0.1", port), H).serve_forever()
PY
  PROXY=$!
  disown "$PROXY" 2>/dev/null
  trap 'kill $PROXY 2>/dev/null' EXIT
  for _ in $(seq 1 30); do curl -s --max-time 1 "http://127.0.0.1:$PPORT/api/health" >/dev/null 2>&1 && break; sleep 0.2; done

  hook_env=(env -i HOME="$FHOME" PATH="$WORKP/bin:/usr/bin:/bin" TMPDIR="$WORKP" LC_ALL=en_US.UTF-8
    LINGGEN_HOST=127.0.0.1 LINGGEN_PORT="$PPORT" LINGGEN_NO_ENGINE_INSTALL=1
    LINGGEN_UPKEEP_CACHE_MIN=0)
  sent() { grep "\"$1\"" "$LOG" | tail -1 | jq -c '.params.arguments' 2>/dev/null; }

  # run_hooks HOST PLUGIN-ROOT RUN-DIR — the three hooks as HOST launches them.
  run_hooks() {
    local host="$1" root="$2" rundir="$3" out
    : >"$LOG"
    out="$(cd "$rundir" && printf '%s' "$(jq -nc --arg c "$PROJ/story" '{session_id:"pc-1", hook_event_name:"SessionStart", source:"startup", cwd:$c}')" \
      | "${hook_env[@]}" CLAUDE_PLUGIN_ROOT="$root" ${CC_PROJECT:+CLAUDE_PROJECT_DIR="$PROJ"} bash "$root/hooks/autostart.sh" 2>&1)"
    expect "$host autostart: valid JSON, core + candidates + index" "$out" \
      '.hookSpecificOutput.hookEventName == "SessionStart" and (.hookSpecificOutput.additionalContext | test("## Core memory") and test("Memory scopes here") and test("## Index — "))' \
      "SessionStart context incomplete"
    expect "$host autostart: sends op=session_start, cwd + root" "$(sent session_start)" \
      '.op == "session_start" and .cwd == "'"$PROJ/story"'" and .root == "'"$PROJ"'"' "session_start args"
    out="$(cd "$rundir" && jq -nc --arg c "$PROJ" '{session_id:"pc-1", hook_event_name:"UserPromptSubmit", prompt:"what is the central artifact in the nine bronze cauldrons novel?", cwd:$c}' \
      | "${hook_env[@]}" CLAUDE_PLUGIN_ROOT="$root" bash "$root/hooks/recall.sh" 2>&1)"
    if printf '%s' "$out" | grep -q '^From memory (.*from=\(user\|derived\|agent\).*id=' && printf '%s' "$out" | grep -q 'Memory capture:'; then
      pass "$host recall: lines carry from=, capture nudge" "$(printf '%s' "$out" | grep -c '^From memory') hits"
    else
      fail "$host recall: lines carry from=, capture nudge" "$(printf '%s' "$out" | head -c 200 | tr '\n' ' ')"
    fi
    expect "$host recall: scope_root sent (not cwd_scope)" "$(sent memory_search)" \
      '.scope_root == "'"$PROJ"'" and (has("cwd_scope") | not)' "search args"
    if printf '%s' "$out" | grep -q 'Sanji'; then fail "$host recall: no cross-project rows" "a sanji row surfaced"
    else pass "$host recall: no cross-project rows"; fi
  }
  CC_PROJECT=1 run_hooks "CC" "$CCP" "$PROJ"
  # Codex: hooks from the plugin root it installed, run from an unrelated cwd,
  # no CLAUDE_PROJECT_DIR.
  CC_PROJECT="" run_hooks "Codex" "$CCP" "/"

  # stamp-cwd.sh with PreToolUse payloads.
  stamp() { # tool tool_input-json cwd
    jq -nc --arg t "$1" --argjson i "$2" --arg c "$3" '{tool_name:$t, session_id:"pc-1", cwd:$c, hook_event_name:"PreToolUse", tool_input:$i}' \
      | "${hook_env[@]}" ${STAMP_PROJECT:+CLAUDE_PROJECT_DIR="$STAMP_PROJECT"} bash "$CCP/hooks/stamp-cwd.sh" 2>&1
  }
  STAMP_PROJECT="$PROJ"
  T=mcp__plugin_linggen_linggen__memory_add
  out="$(stamp "$T" '{"content":"x","scope":"~/workspace","summary":"s","indexed":false}' "$PROJ/story")"
  expect "stamp add: fills cwd/root/source_session/host" "$out" \
    '.hookSpecificOutput.permissionDecision == "allow" and (.hookSpecificOutput.updatedInput | .cwd == "'"$PROJ/story"'" and .root == "'"$PROJ"'" and .source_session == "pc-1" and .host == "claude-code")' \
    "missing stamps"
  expect "stamp add: scope ~, summary, indexed:false intact" "$out" \
    '.hookSpecificOutput.updatedInput | .scope == "~/workspace" and .summary == "s" and .indexed == false and (.indexed | type) == "boolean"' \
    "model fields changed"
  # No output = nothing to stamp = the call goes through as the caller wrote it.
  out="$(stamp "$T" '{"content":"x","cwd":"/elsewhere","root":"/elsewhere","source_session":"other","host":"codex"}' "$PROJ")"
  [ -n "$out" ] || out='{"hookSpecificOutput":{"updatedInput":{"cwd":"/elsewhere","root":"/elsewhere","source_session":"other","host":"codex"}}}' 
  expect "stamp add: never overwrites caller values" "$out" \
    '(.hookSpecificOutput.updatedInput | .cwd == "/elsewhere" and .root == "/elsewhere" and .source_session == "other" and .host == "codex")' \
    "caller value overwritten"
  out="$(stamp "$T" '{"content":"x","source_session":"another-session"}' "$PROJ")"
  expect "stamp add: foreign session keeps its own scope" "$out" \
    '.hookSpecificOutput.updatedInput | (has("cwd") | not) and (has("root") | not) and .source_session == "another-session"' \
    "this session's cwd stamped onto another session's row"
  out="$(stamp mcp__plugin_linggen_linggen__memory_search '{"query":"q"}' "$PROJ/story")"
  expect "stamp search: scope_root = root" "$out" '.hookSpecificOutput.updatedInput | .scope_root == "'"$PROJ"'" and (has("cwd_scope") | not)' "scope_root"
  out="$(stamp mcp__plugin_linggen_linggen__memory_search '{"query":"q","scope_root":"/mine"}' "$PROJ")"
  [ -z "$out" ] && pass "stamp search: caller scope_root kept" || fail "stamp search: caller scope_root kept" "$out"
  # A session started at $HOME (no project dir): a whole-store lookup.
  out="$(STAMP_PROJECT="" stamp mcp__plugin_linggen_linggen__memory_search '{"query":"q"}' "$FHOME")"
  [ -z "$out" ] && pass "stamp search: \$HOME stays a whole-store lookup" || fail "stamp search: \$HOME stays a whole-store lookup" "$out"
  out="$(stamp mcp__plugin_linggen_linggen__memory_upkeep '{"op":"session_start"}' "$PROJ/story")"
  expect "stamp upkeep session_start: cwd + root" "$out" '.hookSpecificOutput.updatedInput | .op == "session_start" and .cwd == "'"$PROJ/story"'" and .root == "'"$PROJ"'"' "session_start stamp"
  # The door's schemas are strict: a stamped field an op does not take is refused.
  out="$(stamp mcp__plugin_linggen_linggen__memory_upkeep '{"op":"days","undreamed_only":true}' "$PROJ/story")"
  [ -z "$out" ] && pass "stamp upkeep: other ops untouched" || fail "stamp upkeep: other ops untouched" "$out"
  out="$(stamp mcp__plugin_linggen_linggen__memory_search '{"op":"get","id":"x"}' "$PROJ/story")"
  [ -z "$out" ] && pass "stamp search op=get: untouched" || fail "stamp search op=get: untouched" "$out"
  out="$(stamp mcp__plugin_linggen_linggen__memory_search '{"op":"list","day":"2026-11-01"}' "$PROJ/story")"
  [ -z "$out" ] && pass "stamp search op=list: untouched (a day's worklist is never narrowed)" || fail "stamp search op=list: untouched" "$out"
  if [ -n "$SKILL" ]; then
    out="$(stamp "$T" '{"content":"x"}' "$SKILL/sub")"
    expect "stamp add: skill dir is its own root" "$out" '.hookSpecificOutput.updatedInput.root == "'"$SKILL"'"' "skill root"
  fi
  # A session started in a non-git workspace, shell cd'd into a nested repo:
  # root stays the workspace the candidates were shown against (the
  # 2026-10-05 bug stamped the nested repo's git root and the daemon dropped
  # the model's scope for cwd). A cwd outside the start dir roots on its own.
  WS="$(dirname "$PROJ")" SIB=""
  for d in "$WS"/*; do
    [ -d "$d/.git" ] && [ "$d" != "$PROJ" ] && { SIB="$d"; break; }
  done
  if [ -n "$SIB" ] && [ ! -e "$WS/.git" ]; then
    out="$(STAMP_PROJECT="$WS" stamp "$T" '{"content":"x","scope":"workspace/lingjing"}' "$SIB")"
    expect "stamp add: nested repo keeps the session root" "$out" \
      '.hookSpecificOutput.updatedInput | .root == "'"$WS"'" and .cwd == "'"$SIB"'" and .scope == "workspace/lingjing"' "root = nested git root"
    out="$(STAMP_PROJECT="$WS" stamp mcp__plugin_linggen_linggen__memory_search '{"query":"q"}' "$PROJ/story")"
    expect "stamp search: nested repo keeps the session root" "$out" \
      '.hookSpecificOutput.updatedInput.scope_root == "'"$WS"'"' "scope_root = nested git root"
    out="$(STAMP_PROJECT="$PROJ" stamp "$T" '{"content":"x"}' "$SIB")"
    expect "stamp add: cwd outside the start dir roots on its own" "$out" \
      '.hookSpecificOutput.updatedInput.root == "'"$SIB"'"' "foreign cwd took the start root"
  else
    skip "stamp add: nested repo keeps the session root" "no sibling repo under a non-git $WS"
  fi
  out="$(stamp mcp__plugin_linggen_linggen__memory_delete '{"id":"x"}' "$PROJ")"
  [ -z "$out" ] && pass "stamp: other tools untouched" || fail "stamp: other tools untouched" "$out"

  kill "$PROXY" 2>/dev/null
else
  skip "hooks against a scratch Linggen" "pass --port --home --project (a scratch Linggen and a fixture home)"
fi

# ── 5. Protocol text alignment ──────────────────────────────────────────────
#
# Stale v1 terms on any surface a model or a host reads. Each pattern is a
# claim the v2 protocol no longer makes; a surface that still makes it teaches
# the old store. Code that keeps an alias on purpose (cwd_scope) is matched
# only where it is presented as the name to use.

ENGINE="$REPO"
SURFACES=(
  "$CCP/skills/linggen" "$CCP/commands" "$CCP/README.md" "$CCP/hooks"
)
ENGINE_SURFACES=(
  "$ENGINE/src/server/mcp" "$ENGINE/crates/linggen-engine/src/engine/tools/memory_mcp.rs"
  "$ENGINE/crates/linggen-memory/src/http/mcp.rs" "$ENGINE/crates/linggen-memory/src/catalog" "$ENGINE/agents/memory.md"
)
for m in "$ENGINE"/missions/*dream* "$ENGINE"/agents/missions/*dream*; do
  [ -e "$m" ] && ENGINE_SURFACES+=("$m")
done
STALE=(
  'cwd_scope[^|]*(the|is|=) *(session|root|scope)|pass `?cwd_scope'
  '\bhook\b[^.]{0,40}(≤ ?80|one[ -]line([^r]|$)|summary line)'
  '"contexts"|`contexts`|contexts_any|--context\b'
  '"tags"|`tags`|--tag\b'
  '[Ss]tanding rules? (load|loaded|block|section)|## Standing rules'
  '[Rr]eview queue[^.]{0,80}(scope|index|summary|hook)( |,|\.)'
  'hook required|requires a hook|hook is required'
  # The retired memory daemon, presented as current: its port, its verbs, its
  # CLI name as the thing to run. (Lines that say it is retired are let through.)
  '127\.0\.0\.1:9528|:9528/mcp|`ling-mem [a-z-]+|ling-mem (daemon|start|upgrade|console)|Bash ling-mem'
)
scan() { # label paths… — print every stale line; FAIL when there is one
  local label="$1"; shift
  local hits="" p
  for p in "${STALE[@]}"; do
    hits="$hits$(grep -rnIE "$p" "$@" 2>/dev/null | grep -vE 'not queued|no longer|retired|dropped|removed|was `?cwd|alias|Was `|first shipped as|Removed|replaced|not by|assert|Historical|/skills/linggen/scripts/install\.sh:|/doc/shared-memory-design\.md:' || true)"$'\n'
  done
  hits="$(printf '%s' "$hits" | sed '/^$/d' | sort -u)"
  if [ -z "$hits" ]; then pass "text: $label" "no stale terms"
  else
    fail "text: $label" "$(printf '%s\n' "$hits" | wc -l | tr -d ' ') stale line(s)"
    printf '%s\n' "$hits" | sed "s|$REAL_HOME|~|; s/^/        /" | head -20
  fi
}
scan "plugins + skill" "${SURFACES[@]}"
ENG=()
for p in "${ENGINE_SURFACES[@]}"; do [ -e "$p" ] && ENG+=("$p"); done
scan "engine surfaces (MCP text, memory agent, dream mission)" "${ENG[@]}"

[ "$N_FAIL" -eq 0 ]
