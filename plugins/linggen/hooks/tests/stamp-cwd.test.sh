#!/usr/bin/env bash
# stamp-cwd.sh stamps only the fields each memory op takes: the door's
# schemas are strict, so a field an op does not take would be refused.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
# No engine answers these tests: not the one this Mac runs on 9527, whose
# [memory] mode would change what the hooks do (mcp.sh: client.json, then
# LINGGEN_HOST/PORT, then 127.0.0.1:9527).
export LINGGEN_DATA_DIR="$(mktemp -d)/.linggen" LINGGEN_HOST=127.0.0.1 LINGGEN_PORT=9
stamp="$here/../stamp-cwd.sh"
# Not under a temp dir: a temp dir is never a project, so nothing would be
# scoped there.
tmp="$(mktemp -d "$here/.tmp.XXXXXX")" || exit 1
trap 'rm -rf "$tmp"' EXIT
proj="$tmp/proj"
mkdir -p "$proj/.git" "$proj/sub"
export HOME="$tmp/home"
mkdir -p "$HOME"
unset CLAUDE_PROJECT_DIR LINGGEN_CLIENT LINGGEN_STAMP_CWD_DISABLE

fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
# call <tool> <tool_input JSON> — the hook's stdout
call() {
  jq -nc --arg t "$1" --argjson i "$2" --arg c "$proj/sub" \
    '{tool_name:$t, tool_input:$i, cwd:$c, session_id:"s1"}' | bash "$stamp"
}
# stamped <tool> <input> <jq test on updatedInput>
stamped() {
  local out
  out="$(call "$1" "$2")" || fail "$1 $2: exit"
  printf '%s' "$out" | jq -e --arg r "$proj" --arg c "$proj/sub" \
    ".hookSpecificOutput.permissionDecision == \"allow\" and (.hookSpecificOutput.updatedInput | $3)" >/dev/null \
    || fail "$1 $2: got $out"
}
# untouched <tool> <input> — no output at all: the call goes as the model wrote it
untouched() {
  local out
  out="$(call "$1" "$2")" || fail "$1 $2: exit"
  [ -z "$out" ] || fail "$1 $2 must not be stamped: got $out"
}

ns=mcp__plugin_linggen_linggen__

# memory_add: cwd, root, source_session, host
stamped "${ns}memory_add" '{"content":"x"}' \
  '. == {content:"x", source_session:"s1", cwd:$c, root:$r, host:"claude-code"}'
# a promote carrying another session's row: only host is added
stamped "${ns}memory_add" '{"content":"x","source_session":"other","cwd":"/elsewhere"}' \
  '. == {content:"x", source_session:"other", cwd:"/elsewhere", host:"claude-code"}'
# a field the caller set wins
stamped "${ns}memory_add" '{"content":"x","host":"openclaw","root":"/r"}' \
  '.host == "openclaw" and .root == "/r" and .cwd == $c'
# Codex: only the host stamp differs (its tool name is mcp__<server>__<tool>)
out="$(jq -nc --arg c "$proj/sub" '{tool_name:"mcp__linggen__memory_add", tool_input:{content:"x"}, cwd:$c, session_id:"s1"}' \
  | LINGGEN_CLIENT=codex bash "$stamp")"
printf '%s' "$out" | jq -e '.hookSpecificOutput.updatedInput | .host == "codex" and .source_session == "s1"' >/dev/null \
  || fail "Codex add: got $out"

# memory_search: scope_root on semantic (op omitted or named) and on a list
# that names no day; a day's worklist and a get are never narrowed
stamped "${ns}memory_search" '{"query":"q"}' '. == {query:"q", scope_root:$r}'
stamped "${ns}memory_search" '{"op":"semantic","query":"q"}' '. == {op:"semantic", query:"q", scope_root:$r}'
untouched "${ns}memory_search" '{"op":"list","day":"2026-11-01"}'
stamped "${ns}memory_search" '{"op":"list","tier":"semantic"}' '. == {op:"list", tier:"semantic", scope_root:$r}'
untouched "${ns}memory_search" '{"query":"q","scope_root":"/mine"}'
untouched "${ns}memory_search" '{"op":"get","id":"abc"}'

# memory_upkeep: cwd and root on session_start only
stamped "${ns}memory_upkeep" '{"op":"session_start"}' '. == {op:"session_start", cwd:$c, root:$r}'
for op in days harvest_day remember_day dream_lease role sweep issues issue_add issue_resolve chains dream_status dream_run; do
  untouched "${ns}memory_upkeep" "{\"op\":\"$op\"}"
done
untouched "${ns}memory_upkeep" '{}'

# every other tool: nothing
untouched "${ns}memory_update" '{"id":"abc","content":"y"}'
untouched "${ns}memory_delete" '{"id":"abc"}'
untouched "${ns}agent_run" '{"prompt":"p"}'

# outside a project, a search is a whole-store lookup: no scope_root
out="$(jq -nc --arg c "$HOME" '{tool_name:"mcp__plugin_linggen_linggen__memory_search", tool_input:{query:"q"}, cwd:$c, session_id:"s1"}' | bash "$stamp")"
[ -z "$out" ] || fail "search at HOME must not be scoped: got $out"

printf 'ok: stamp-cwd stamps each op only with the fields it takes\n'
