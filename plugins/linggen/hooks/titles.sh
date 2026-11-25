#!/usr/bin/env bash
# Sourced by recall.sh under v2 (`outside_agents = "v2"`): the per-turn lookup
# of Linggen's own notes, TITLES ONLY. What this host remembers it keeps
# itself; Linggen's notes (decisions, rules, gotchas the person gave any
# agent) reach it at session start (autostart.sh) and, here, when the
# prompt's own words match a note's summary or topic.
#
# The engine decides what matches and what a coding host may see
# (`memory_notes op=titles`: plain words, no vectors, the privacy fence);
# this only asks, prints at most a few lines, and remembers which ids this
# session was already shown so none is repeated. Silent when the engine
# does not answer in time or nothing matches — never blocks the prompt.
#
# Needs from the caller: mcp_call (mcp.sh), jq, $prompt, $input (the hook's
# JSON, for the session id).

# titles_for_prompt — prints "相关笔记: <summary> (<id>)" lines.
titles_for_prompt() {
  local limit="${LINGGEN_TITLES_LIMIT:-4}" to="${LINGGEN_TITLES_TIMEOUT:-0.4}"
  local sid dir seen_file seen_json args out lines
  sid="$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null | tr -c 'A-Za-z0-9_-' '_')"
  dir="${TMPDIR:-/tmp}/linggen-titles-$(id -u)"
  seen_file=""
  seen_json='[]'
  if [ -n "$sid" ]; then
    mkdir -p "$dir" 2>/dev/null && seen_file="$dir/$sid"
    [ -n "$seen_file" ] && [ -f "$seen_file" ] && seen_json="$(jq -Rsc 'split("\n") | map(select(length > 0))' "$seen_file" 2>/dev/null || printf '[]')"
  fi
  args="$(jq -nc --arg w "$prompt" --argjson l "$limit" --argjson x "$seen_json" \
    '{op:"titles", words:$w, limit:$l, exclude:$x}' 2>/dev/null)" || return 0
  out="$(mcp_call memory_notes "$args" "$to")"
  [ -n "$out" ] || return 0
  lines="$(printf '%s' "$out" | jq -r '(.titles // [])[] | "相关笔记: \(.summary) (\(.id))"' 2>/dev/null || true)"
  [ -n "$lines" ] || return 0
  printf '%s\n' "$lines"
  if [ -n "$seen_file" ]; then
    printf '%s' "$out" | jq -r '(.titles // [])[].id' >>"$seen_file" 2>/dev/null || true
    # A day's worth of sessions is plenty to remember.
    find "$dir" -type f -mtime +1 -delete 2>/dev/null || true
  fi
}
