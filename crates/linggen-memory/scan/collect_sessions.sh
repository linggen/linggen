#!/usr/bin/env bash
#
# Privacy + intent (for human readers and static scanners):
# Walks LOCAL session-file directories and prints metadata (paths, byte
# size, message counts) to stdout. Reads filenames + a few JSONL entries
# per file to count messages. No network calls. No file content leaves
# the machine. Used by the agent to decide which sessions are worth
# extracting durable preferences from.
#
# collect_sessions.sh — Scan host session stores for a date.
#
# Hosts covered (per design doc §4):
#   - Claude Code: ~/.claude/projects/<enc-cwd>/<uuid>.jsonl
#   - Codex:       ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl (+ archived_sessions/)
#   - OpenClaw:    ~/.openclaw/logs/*.jsonl
#   - Linggen:     ~/.linggen/sessions/<id>/messages.jsonl  (legacy host install)
#
# Emits NDJSON (one object per session) to stdout:
#   {"filepath":"...","source":"CC"|"Codex"|"OpenClaw"|"Linggen","label":"...",
#    "date":"...","bytes":N,"user_turns":N}
#
# user_turns counts real user messages (skips tool_result-only "user" entries in
# CC / Codex). Callers can skip empty/greeting-only sessions before judging.
#
# Per-source watermark (--watermark <file>): if provided, emit only files whose
# mtime is strictly newer than the per-source ISO-8601 timestamp recorded in
# the watermark file. The file is JSON of the form:
#   { "CC": "<ISO>", "Codex": "<ISO>", "OpenClaw": "<ISO>", "Linggen": "<ISO>" }
# Missing keys = no filter for that source. Updating the watermark is the
# caller's job (Phase 4 of the dream flow).
#
# Usage: ./collect_sessions.sh [YYYY-MM-DD] [--watermark <path>]
#
# Requires: jq

set -uo pipefail

TARGET_DATE=""
WATERMARK_FILE=""
OTHER_COUNT_FILE=""
SKIPPED_OTHER=0

if [[ -z "${LINGGEN_SCAN_OWNER_ID+x}" || -z "${LINGGEN_SCAN_LOCAL_ID:-}" ]]; then
  echo 'ling mem scan: missing owner identity handoff' >&2
  exit 1
fi

while [ "$#" -gt 0 ]; do
  case "$1" in
    --watermark) WATERMARK_FILE="${2:-}"; shift 2 ;;
    --watermark=*) WATERMARK_FILE="${1#--watermark=}"; shift ;;
    --other-count-file) OTHER_COUNT_FILE="$2"; shift 2 ;;
    *) if [ -z "$TARGET_DATE" ]; then TARGET_DATE="$1"; fi; shift ;;
  esac
done

TARGET_DATE="${TARGET_DATE:-$(date +%Y-%m-%d)}"

HOME_DIR="${HOME:-$(eval echo ~)}"
CC_DIR="${HOME_DIR}/.claude/projects"
CODEX_DIR="${HOME_DIR}/.codex/sessions"
CODEX_ARCHIVE="${HOME_DIR}/.codex/archived_sessions"
OPENCLAW_DIR="${HOME_DIR}/.openclaw/agents"
LING_HOME="${LINGGEN_HOME:-${HOME_DIR}/.linggen}"
LING_DIR="$LING_HOME/sessions"

if ! command -v jq &>/dev/null; then
  echo "Error: jq is required but not installed." >&2
  exit 1
fi

# ── watermark helpers ────────────────────────────────────────────────
watermark_for() {
  # Echo the ISO-8601 timestamp recorded for <source>, or empty.
  local src="$1"
  [ -n "$WATERMARK_FILE" ] && [ -f "$WATERMARK_FILE" ] || return 0
  jq -r --arg s "$src" '.[$s] // ""' "$WATERMARK_FILE" 2>/dev/null
}

file_mtime_epoch() {
  if [[ "$(uname)" == "Darwin" ]]; then
    stat -f "%m" "$1" 2>/dev/null || echo 0
  else
    stat -c "%Y" "$1" 2>/dev/null || echo 0
  fi
}

iso_to_epoch() {
  # ISO-8601 (UTC, with or without trailing Z) → seconds since epoch.
  local iso="$1"
  [ -n "$iso" ] || { echo 0; return; }
  if [[ "$(uname)" == "Darwin" ]]; then
    date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "${iso%Z}Z" "+%s" 2>/dev/null \
      || date -u -j -f "%Y-%m-%dT%H:%M:%S" "${iso%Z}" "+%s" 2>/dev/null \
      || echo 0
  else
    date -u -d "$iso" "+%s" 2>/dev/null || echo 0
  fi
}

# Returns 0 if file mtime > watermark for source; 1 otherwise.
past_watermark() {
  local src="$1" file="$2"
  local wm; wm="$(watermark_for "$src")"
  [ -n "$wm" ] || return 0
  local wm_epoch; wm_epoch="$(iso_to_epoch "$wm")"
  local f_epoch;  f_epoch="$(file_mtime_epoch "$file")"
  [ "$f_epoch" -gt "$wm_epoch" ]
}

# ── byte / turn counting ─────────────────────────────────────────────
count_user_turns_cc() {
  # CC: type=="user" with a text content block (tool_result entries don't count)
  jq -s '[.[] | select(.type == "user") | .message.content
         | if type == "array"
           then map(select(.type == "text" and (.text // "" | length > 0))) | length
           else (if . != null and . != "" then 1 else 0 end) end]
         | add // 0' "$1" 2>/dev/null || echo 0
}

count_user_turns_codex() {
  # Codex rollout, in EITHER of the two shapes the format has used:
  #
  #   newer  {"type":"response_item","payload":{"role":"user","content":[…]}}
  #   older  {"type":"message","role":"user","content":[…]}          (flat)
  #
  # Only the wrapped shape was matched before, so every flat-format rollout
  # counted zero and the scan skipped it — two files of 62 KB and 83 KB on this
  # machine alone, holding 16 and 23 real user messages. A counter that reads a
  # transcript as empty is indistinguishable from a transcript with nothing in
  # it, which is why this failed silently for months.
  #
  # Both shapes carry the same `input_text` content blocks; tool-result entries
  # have another type and still don't count.
  jq -s '[.[]
          | (if   .type == "response_item" then .payload
             elif .type == "message"       then .
             else empty end) as $m
          | select(($m.role // "") == "user")
          | $m.content
          | if type == "array"
            then map(select((.type // "") == "input_text"
                             and (.text // "" | length > 0))) | length
            else 0 end]
         | add // 0' "$1" 2>/dev/null || echo 0
}

count_user_turns_openclaw() {
  # OpenClaw: lines with `type=="message"` where `.message.role == "user"`.
  # (Older script looked for top-level `.role`, which never matched the
  # `agents/<name>/sessions/<uuid>.jsonl` schema — every session returned 0.)
  jq -s '[.[] | select(.type == "message"
                       and (.message.role // "") == "user")] | length' "$1" 2>/dev/null || echo 0
}

# A top-level scalar of a session.yaml (`creator`, `skill`), unquoted.
yaml_field() {
  grep -m1 "^$2:" "$1" 2>/dev/null \
    | sed -E "s/^$2:[[:space:]]*//; s/[[:space:]]*#.*\$//; s/^\"(.*)\"\$/\\1/; s/^'(.*)'\$/\\1/"
}

# Whether an app keeps its talk to itself: `private-talk: true` in its
# SKILL.md's front matter. An app that is not installed is treated so too —
# what cannot be asked is not handed on.
keeps_its_talk() {
  local skill_md="$LING_HOME/skills/$1/SKILL.md"
  [ -f "$skill_md" ] || return 0
  sed -n '2,/^---[[:space:]]*$/p' "$skill_md" | grep -Eq '^private-talk:[[:space:]]*true[[:space:]]*$'
}

count_user_turns_linggen() {
  # `[HIDDEN]`-prefixed rows are the app shell talking to the agent, not the
  # person: skill pages fire a greeting/context trigger through the same send
  # path the user types into (EmbedApp prefixes the text, the engine strips the
  # prefix, and five server read sites filter it back out). Counting them makes
  # a session nobody spoke in look busy, so scan.sh's empty-session filter never
  # drops it and the judge reads our own boilerplate as the user's words.
  #
  # Anchored with startswith, NOT contains: the marker also appears mid-string
  # in real content ("Starting autonomous loop for task: [HIDDEN] …"), so a
  # substring test would eat a genuine message that merely quotes the token.
  jq -s '[.[] | select(.from_id == "user")
                | select(((.content // "") | startswith("[HIDDEN]")) | not)]
         | length' "$1" 2>/dev/null || echo 0
}

file_bytes() {
  if [[ "$(uname)" == "Darwin" ]]; then
    stat -f "%z" "$1" 2>/dev/null || echo 0
  else
    stat -c "%s" "$1" 2>/dev/null || echo 0
  fi
}

emit_manifest() {
  # emit_manifest <filepath> <source> <label> <bytes> <user_turns>
  jq -cn --arg filepath "$1" \
         --arg source "$2" \
         --arg label "$3" \
         --arg date "$TARGET_DATE" \
         --argjson bytes "$4" \
         --argjson user_turns "$5" \
         '{filepath:$filepath, source:$source, label:$label, date:$date,
           bytes:$bytes, user_turns:$user_turns}'
}

file_date_str() {
  if [[ "$(uname)" == "Darwin" ]]; then
    stat -f "%Sm" -t "%Y-%m-%d" "$1" 2>/dev/null || echo ""
  else
    date -r "$1" +%Y-%m-%d 2>/dev/null || echo ""
  fi
}

# ── Claude Code ──────────────────────────────────────────────────────
if [ -d "$CC_DIR" ]; then
  for project_dir in "$CC_DIR"/*/; do
    [ -d "$project_dir" ] || continue
    for jsonl_file in "$project_dir"*.jsonl; do
      [ -f "$jsonl_file" ] || continue

      file_date="$(file_date_str "$jsonl_file")"
      [[ "$file_date" == "$TARGET_DATE" ]] || continue
      past_watermark "CC" "$jsonl_file" || continue

      project_name=$(basename "$project_dir")
      session_name=$(basename "$jsonl_file" .jsonl)
      label="${project_name}/${session_name}"
      bytes=$(file_bytes "$jsonl_file")
      user_turns=$(count_user_turns_cc "$jsonl_file")
      emit_manifest "$jsonl_file" "CC" "$label" "${bytes:-0}" "${user_turns:-0}"
    done
  done
fi

# ── Codex (rollouts + archived sessions) ─────────────────────────────
collect_codex_dir() {
  local root="$1"
  [ -d "$root" ] || return 0
  # Codex partitions by YYYY/MM/DD; walk all rollout-*.jsonl files under root.
  while IFS= read -r jsonl_file; do
    [ -f "$jsonl_file" ] || continue
    file_date="$(file_date_str "$jsonl_file")"
    [[ "$file_date" == "$TARGET_DATE" ]] || continue
    past_watermark "Codex" "$jsonl_file" || continue

    rel_path="${jsonl_file#$root/}"
    label="${rel_path}"
    bytes=$(file_bytes "$jsonl_file")
    user_turns=$(count_user_turns_codex "$jsonl_file")
    emit_manifest "$jsonl_file" "Codex" "$label" "${bytes:-0}" "${user_turns:-0}"
  done < <(find "$root" -type f -name 'rollout-*.jsonl' 2>/dev/null)
}
collect_codex_dir "$CODEX_DIR"
collect_codex_dir "$CODEX_ARCHIVE"

# ── OpenClaw ─────────────────────────────────────────────────────────
# Session JSONLs live at ~/.openclaw/agents/<agent>/sessions/<uuid>.jsonl.
# Skip the companion `<uuid>.trajectory.jsonl` files — those are structured
# trace events, not chat turns, and have a different schema.
if [ -d "$OPENCLAW_DIR" ]; then
  while IFS= read -r jsonl_file; do
    [ -f "$jsonl_file" ] || continue
    file_date="$(file_date_str "$jsonl_file")"
    [[ "$file_date" == "$TARGET_DATE" ]] || continue
    past_watermark "OpenClaw" "$jsonl_file" || continue

    label="$(basename "$jsonl_file" .jsonl)"
    bytes=$(file_bytes "$jsonl_file")
    user_turns=$(count_user_turns_openclaw "$jsonl_file")
    emit_manifest "$jsonl_file" "OpenClaw" "$label" "${bytes:-0}" "${user_turns:-0}"
  done < <(find "$OPENCLAW_DIR" -type f -name '*.jsonl' ! -name '*.trajectory.jsonl' 2>/dev/null)
fi

# ── Linggen (legacy host install) ────────────────────────────────────
# Use only the identities the host supplied; an empty account means signed out.
other_person() {
  local id
  id="$(yaml_field "$1" user_id || yaml_field "$1" consumer_user_id)"
  id="${id#"${id%%[![:space:]]*}"}"
  id="${id%"${id##*[![:space:]]}"}"
  [[ -n "$id" && "$id" != "$LINGGEN_SCAN_LOCAL_ID" && "$id" != "$LINGGEN_SCAN_OWNER_ID" ]]
}

if [ -d "$LING_DIR" ]; then
  for session_dir in "$LING_DIR"/*/; do
    [ -d "$session_dir" ] || continue
    jsonl_file="${session_dir}messages.jsonl"
    [ -f "$jsonl_file" ] || continue

    file_date="$(file_date_str "$jsonl_file")"
    [[ "$file_date" == "$TARGET_DATE" ]] || continue
    past_watermark "Linggen" "$jsonl_file" || continue

    session_name=$(basename "$session_dir")
    meta="${session_dir}session.yaml"
    [ -f "$meta" ] || continue

    # The rule: a session the person spoke in is read, whoever made it —
    # their own chats, Linggen's daily session, the companion's thread, an
    # app's own chat. Four kinds stay out:
    #   - another person's day or ordinary chat;
    #   - a mission's session and a session a host made through agent_run:
    #     their `user` rows are the job's or the host's prompt, not a
    #     person's words (what the person told that host is read from the
    #     host's own logs);
    #   - a private app's chat (`private-talk: true`), or an app that cannot
    #     be found — its talk is kept to itself.
    if [[ "$session_name" =~ ^sess-day-[0-9]{4}-[0-9]{2}-[0-9]{2}-p- ]] || other_person "$meta"; then
      SKIPPED_OTHER=$((SKIPPED_OTHER + 1))
      continue
    fi
    case "$(yaml_field "$meta" creator)" in mission|external) continue ;; esac
    skill="$(yaml_field "$meta" skill)"
    if [ -n "$skill" ] && keeps_its_talk "$skill"; then continue; fi
    user_turns=$(count_user_turns_linggen "$jsonl_file")
    [ "${user_turns:-0}" -gt 0 ] || continue

    session_title=$(grep '^title:' "$meta" 2>/dev/null | sed 's/^title: *//' | head -1)
    label="${session_title:-$session_name}"
    bytes=$(file_bytes "$jsonl_file")
    emit_manifest "$jsonl_file" "Linggen" "$label" "${bytes:-0}" "${user_turns:-0}"
  done
fi

if [[ -n "$OTHER_COUNT_FILE" ]]; then
  printf '%s\n' "$SKIPPED_OTHER" >> "$OTHER_COUNT_FILE"
fi
