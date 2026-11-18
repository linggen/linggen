#!/usr/bin/env bash
# Which of Linggen's own sessions the nightly memory scan reads: a session
# the person spoke in, whoever made it — never another person's day, a
# mission's or a host-made session, or a private app's chat.
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
skills="$HOME/.linggen/skills"
day="sess-day-$scan_day"
ordinary=sess-1794312000-a1e7a1e7

message() {
  jq -cn --arg from "$1" --arg text "$2" --arg place "${3:-:home}" \
    --argjson ts "${4:-1794312000}" --argjson observation "${5:-false}" \
    '{from_id:$from, content:$text, place:$place, timestamp:$ts, is_observation:$observation}'
}

# session <id> <creator> [<skill>]: its messages come on stdin.
session() {
  local dir="$sessions/$1"
  mkdir -p "$dir"
  printf 'creator: %s\n' "$2" > "$dir/session.yaml"
  [ -z "${3:-}" ] || printf 'skill: %s\n' "$3" >> "$dir/session.yaml"
  cat > "$dir/messages.jsonl"
  touch -t 202611101200 "$dir/messages.jsonl"
}

# app <name> [private]: an installed app, private when asked.
app() {
  mkdir -p "$skills/$1"
  { echo '---'; echo "name: $1"; [ -z "${2:-}" ] || echo 'private-talk: true'; echo '---'; echo; echo 'An app.'; } \
    > "$skills/$1/SKILL.md"
}

chat() {
  message user 'I prefer tea.'
  message ling 'Tea it is.'
  message user 'I take it without sugar.'
}

collected() { bash "$scan_dir/collect_sessions.sh" "$scan_day" | jq -r '.filepath' | sed "s|$sessions/||; s|/messages.jsonl||" | sort | tr '\n' ' '; }

expect() {
  local got; got="$(collected)"
  [[ "$got" == "$1" ]] || { printf 'FAIL: %s\n  expected: %s\n  got:      %s\n' "$2" "$1" "$got" >&2; exit 1; }
}

app tunes
app ledger private

# Nobody's words: none of these is read, though each holds `user` rows.
chat | session "$ordinary" user
expect "$ordinary " 'a chat the person made'

chat | session "$day-p-752d3432" agent                 # another person's day
chat | session "$day-p-752d3433" user
chat | session sess-mission mission                    # a job's own prompt
chat | session sess-from-a-host external               # a host's brief
chat | session sess-ledger skill ledger                # a private app
chat | session sess-gone skill no-such-app             # an app that cannot be asked
{ message user "$(printf '[HIDDEN]%2048s' 'app kickoff')"; message ling 'An automatic greeting.'; } \
  | session sess-tunes-quiet skill tunes               # an app's page spoke; the person did not
{ message user '[HIDDEN] kickoff'; message ling 'A greeting.'; message yinyue 'An unprompted moment.'; } \
  | session "$day" agent                               # the day, before the person spoke
expect "$ordinary " 'a session the person did not speak in, or one kept out, was collected'

# The person's words, whoever made the session: all of these are read.
chat | session sess-tunes skill tunes                  # an app's own chat
chat | session sess-yinyue-$scan_day agent             # the companion's thread
{
  message user '[HIDDEN] app kickoff'
  message user 'I quote [HIDDEN] literally.'
  message ling 'Understood.'
  message user 'In this app I prefer a short list.' tunes
  message ling 'Here is a short list.' tunes
  message yinyue 'An unprompted moment.' tunes
  message yinyue 'A reply from the companion.'
  message compaction 'Summary of trimmed dialogue.'
  message ling 'A tool observation.' :home 1794312000 true
  message ling 'After this calendar day.' :home 1794398400
} | session "$day" agent                               # the day, once the person spoke
expect "$ordinary $day sess-tunes sess-yinyue-$scan_day " 'the person spoke in these four'

turns="$(bash "$scan_dir/collect_sessions.sh" "$scan_day" | jq -r --arg p "$sessions/$day/messages.jsonl" 'select(.filepath == $p) | .user_turns')"
[[ "$turns" == 2 ]] || { echo "FAIL: the day's real user turns: $turns" >&2; exit 1; }

# What is handed on for the day: the person's words and the answers to them.
bash "$scan_dir/scan.sh" "$scan_day" > /dev/null
expected='[user]: I quote [HIDDEN] literally.
[assistant]: Understood.
[user]: In this app I prefer a short list.
[assistant]: Here is a short list.'
jq -es --arg path "$sessions/$day/messages.jsonl" --arg transcript "$expected" \
  'any(.[]; .filepath == $path and .transcript == $transcript)' \
  "$HOME/.linggen/memory/.scan-output.jsonl" > /dev/null || {
  echo 'FAIL: the transcript handed on for the day' >&2
  jq -r --arg path "$sessions/$day/messages.jsonl" 'select(.filepath == $path) | .transcript' "$HOME/.linggen/memory/.scan-output.jsonl" >&2
  exit 1
}

echo 'PASS: a session the person spoke in is read; other people, jobs, hosts and private apps are not'
