#!/usr/bin/env bash
# Script-only fixture beside memory_ops.rs; no engine or real home needed.
# Run with: bash tests/system/memory_scan_day.sh
set -euo pipefail

scan_dir="$(cd "$(dirname "$0")/../../crates/linggen-memory/scan" && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
export HOME="$fixture/home" TMPDIR="$fixture/tmp" TZ=UTC
mkdir -p "$HOME" "$TMPDIR"

scan_day=2026-11-10
sessions="$HOME/.linggen/sessions"
owner="sess-day-$scan_day"
ordinary=sess-1794312000-a1e7a1e7

message() {
  jq -cn --arg from "$1" --arg text "$2" --arg place "${3:-:home}" \
    --argjson ts "${4:-1794312000}" --argjson observation "${5:-false}" \
    '{from_id:$from, content:$text, place:$place, timestamp:$ts, is_observation:$observation}'
}

session() {
  local dir="$sessions/$1"
  mkdir -p "$dir"
  printf 'creator: %s\n' "$2" > "$dir/session.yaml"
  cat > "$dir/messages.jsonl"
  touch -t 202611101200 "$dir/messages.jsonl"
}

chat() {
  message user 'I prefer tea.'
  message ling 'Tea it is.'
  message user 'I take it without sugar.'
}

chat | session "$ordinary" user
baseline="$(bash "$scan_dir/collect_sessions.sh" "$scan_day")"
jq -es --arg path "$sessions/$ordinary/messages.jsonl" \
  'length == 1 and .[0].filepath == $path and .[0].user_turns == 2' \
  <<< "$baseline" > /dev/null

chat | session "$owner-p-752d3432" agent
chat | session "$owner-p-752d3433" user
chat | session "${owner}x" agent
chat | session sess-mission mission
chat | session sess-private-app skill

# Even a transcript large enough to pass scan.sh's legacy byte gate must
# not enter the manifest if the owner never spoke.
{
  message user "$(printf '[HIDDEN]%2048s' 'app kickoff')"
  message ling 'An automatic greeting.'
  message yinyue 'An unprompted moment.'
} | session "$owner" agent
[[ "$(bash "$scan_dir/collect_sessions.sh" "$scan_day")" == "$baseline" ]] || {
  echo 'FAIL: another person, non-user session, or owner day without real user turns was collected' >&2
  exit 1
}

{
  message user '[HIDDEN] app kickoff'
  message user 'I quote [HIDDEN] literally.'
  message ling 'Understood.'
  message user 'In this app I prefer a short list.' shared-app
  message ling 'Here is a short list.' shared-app
  message yinyue 'An unprompted moment.' shared-app
  message yinyue 'A reply from the companion.'
  message compaction 'Summary of trimmed dialogue.'
  message ling 'A tool observation.' :home 1794312000 true
  message ling 'After this calendar day.' :home 1794398400
} | session "$owner" agent

manifest="$(bash "$scan_dir/collect_sessions.sh" "$scan_day")"
jq -es --arg path "$sessions/$owner/messages.jsonl" \
  'length == 2 and any(.[]; .filepath == $path and .user_turns == 2)' \
  <<< "$manifest" > /dev/null || {
  printf 'FAIL: expected the owner day with two real user turns and the ordinary chat; got:\n%s\n' "$manifest" >&2
  exit 1
}

bash "$scan_dir/scan.sh" "$scan_day"
expected='[user]: I quote [HIDDEN] literally.
[assistant]: Understood.
[user]: In this app I prefer a short list.
[assistant]: Here is a short list.'
jq -es --arg path "$sessions/$owner/messages.jsonl" --arg transcript "$expected" \
  'length == 3 and .[0].sessions_scanned == 2
   and any(.[]; .filepath == $path and .transcript == $transcript)' \
  "$HOME/.linggen/memory/.scan-output.jsonl" > /dev/null

echo 'PASS: owner day collection and transcript; other days and app/mission sessions excluded'
