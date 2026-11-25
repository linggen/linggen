#!/usr/bin/env bash
# Stop hook (Claude Code only). Heat: for each row the recall hook pushed this
# turn, count it as recalled, and as used when the reply cites its id (the
# "From memory (…, id=X)" line the hook prints carries it). Text match only —
# no model. Silent on any failure; never blocks. Codex has no Stop hook with a
# transcript, so it has no heat. Decided in linggen-memory `memory::heat`.
set -u
command -v jq   >/dev/null 2>&1 || exit 0
command -v curl >/dev/null 2>&1 || exit 0

input="$(cat)"
session_id="$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null || true)"
transcript="$(printf '%s' "$input" | jq -r '.transcript_path // empty' 2>/dev/null || true)"
[ -n "$session_id" ] || exit 0
pushed_file="${TMPDIR:-/tmp}/linggen-pushed-$(id -u)-${session_id//[^A-Za-z0-9_-]/}"
[ -s "$pushed_file" ] || exit 0
ids="$(cat "$pushed_file" 2>/dev/null || true)"
rm -f "$pushed_file"
[ -n "$ids" ] && [ -r "$transcript" ] || exit 0

# shellcheck source=./mcp.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/mcp.sh" 2>/dev/null || exit 0

# The assistant's text since the last real user prompt (tool results are
# "user" entries too, but carry no text of the user's).
reply="$(tail -n 400 "$transcript" 2>/dev/null | jq -rs '
  map(select(.type == "user" or .type == "assistant")) as $e
  | ([$e | to_entries[]
      | select(.value.type == "user"
               and ((.value.message.content | type) == "string"
                    or any(.value.message.content[]?; .type == "text")))
      | .key] | last // -1) as $u
  | [$e[($u + 1):][] | select(.type == "assistant") | .message.content[]? | select(.type == "text") | .text]
  | join("\n")' 2>/dev/null || true)"

# Each line is "id<TAB>first 30 chars of the row". Used = the reply names the
# id, or quotes the row's opening on a "From memory" line.
while IFS=$'\t' read -r id snippet; do
  [ -n "$id" ] || continue
  used=0
  case "$reply" in *"$id"*) used=1 ;; esac
  if [ "$used" = 0 ] && [ -n "$snippet" ]; then
    case "$reply" in *"From memory"*"$snippet"*) used=1 ;; esac
  fi
  mcp_call memory_update "$(jq -nc --arg i "$id" --argjson u "$used" '{id:$i, bump_recalled:1, bump_used:$u}')" 1 >/dev/null
done <<< "$ids"
exit 0
