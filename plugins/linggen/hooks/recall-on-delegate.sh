#!/usr/bin/env bash
# Offer recall to the caller of a host subagent. Never change the call or
# decide its permission. Linggen's agent_run recalls in its own session.
set -u
[ "${LINGGEN_DELEGATE:-1}" = 0 ] && exit 0
command -v jq >/dev/null 2>&1 || exit 0
input="$(cat)"
query="$(printf '%s' "$input" | jq -er '
  select(.tool_name == "Agent" or .tool_name == "Task") |
  .tool_input.prompt | select(type == "string" and length >= 40 and test("\\s")) | .[:2000]
' 2>/dev/null)" || exit 0
cwd="$(printf '%s' "$input" | jq -r '.cwd // empty' 2>/dev/null)" || exit 0
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)" || exit 0
. "$here/scope.sh" 2>/dev/null || exit 0
scope=""
if [ -n "$cwd" ]; then
  scope="$(session_root "$cwd" 2>/dev/null)" || exit 0
fi
args="$(jq -nc --arg q "$query" --argjson l "${LINGGEN_DELEGATE_LIMIT:-3}" --arg c "$scope" '
  {query:$q, limit:$l} + (if ($c | length) > 0 then {scope_root:$c} else {} end)
' 2>/dev/null)" || exit 0
if [ -n "${LINGGEN_SEARCH_CMD:-}" ]; then
  out="$("$LINGGEN_SEARCH_CMD" memory_search "$args" "${LINGGEN_RECALL_TIMEOUT:-3}" 2>/dev/null)" || exit 0
else
  command -v curl >/dev/null 2>&1 || exit 0
  . "$here/mcp.sh" 2>/dev/null || exit 0
  out="$(mcp_call memory_search "$args" "${LINGGEN_RECALL_TIMEOUT:-3}")" || exit 0
fi
[ -n "$out" ] || exit 0
lines="$(printf '%s' "$out" | jq -r --argjson input "$input" '
  if type != "array" then empty else .[] |
  select((.id | type) == "string" and (.id | length) > 0) |
  select(.id as $id | ($input.tool_input.prompt | contains($id) | not)) |
  "- (\(.type), \((.created_at // "")[0:10]), id=\(.id)) \((.content // "")[0:400] | gsub("[\r\n]+"; " "))"
  end
' 2>/dev/null)" || exit 0
[ -n "$lines" ] || exit 0
jq -nc --arg rows "$lines" '
  {hookSpecificOutput: {hookEventName:"PreToolUse", additionalContext:("Recalled for the task you just delegated:\n" + $rows)}}
' 2>/dev/null || exit 0
exit 0
