#!/usr/bin/env bash
# UserPromptSubmit hook installed by the linggen plugin. Surfaces relevant
# memories for each turn. Bails silently on any failure — never blocks
# the user.
#
# Talks to Linggen's memory over **MCP** (the one door, `/mcp`), not the CLI. A hook
# can't *be* a model's tool call, but it can *make* one: `tools/call` is a
# JSON-RPC POST, and curl can post JSON. What that buys is the whole point —
# this hook needs a URL and (off-machine) a token, and **no binary at all**.
# `ling mem` can only ever reach *this* machine's store, so a machine with only
# the plugin on CLI recall would need its own Linggen and its own store: memory
# forked in two.
#
# Cost, accepted: Linggen must be up. The CLI could fall back to opening the
# store directly; a URL cannot. The SessionStart hook starts it, and a quiet
# turn is the correct failure — recall is an enhancement, never a gate.

set -u
[ "${LINGGEN_RECALL_DISABLE:-0}" = "1" ] && exit 0

command -v jq   >/dev/null 2>&1 || exit 0
command -v curl >/dev/null 2>&1 || exit 0

input="$(cat)"
prompt="$(printf '%s' "$input" | jq -r '.prompt // empty' 2>/dev/null || true)"
cwd="$(printf '%s' "$input"   | jq -r '.cwd    // empty' 2>/dev/null || true)"

[ "${#prompt}" -lt 8 ] && exit 0

topk="${LINGGEN_RECALL_TOPK:-3}"
limit="${LINGGEN_RECALL_LIMIT:-8}"
to="${LINGGEN_RECALL_TIMEOUT:-3}"
# No hardcoded floor: with no `min_score` memory applies its store-wide
# `recall_min_score` (one selectivity shared by all hosts). Set
# LINGGEN_RECALL_MIN_SCORE to tighten it per host — applied client-side,
# because `min_score` is deliberately absent from the MCP schema (a *model*
# guessing a threshold narrows recall to zero). The rows carry their scores,
# so filtering here costs nothing and needs no schema change.
min_score="${LINGGEN_RECALL_MIN_SCORE:-0}"

# Address + `mcp_call`, shared with autostart.sh. Located from this script's
# own path rather than a plugin-root env var — Codex's hook runner doesn't set
# one, and recall has to keep working there.
hooks_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./mcp.sh
. "$hooks_dir/mcp.sh" 2>/dev/null || exit 0

# Scope the search to where this session stands, as its root (git root, or
# where the session started): rows under it, rows at its parents, and rows
# about the person. A skill's own dir sees only its rows; $HOME, ~/.linggen
# and temp dirs see rows about the person plus at most two strong
# non-preference matches filed under a directory. Memory applies it BEFORE
# ranking — a filter applied afterwards can only shrink a list that was
# already the wrong N — and owns the rules (linggen-memory doc/scope-index-spec.md).
# `skip_if_warming`: while memory warms its embedder this search is refused
# at once (empty output) instead of waiting — a recall never holds a prompt.
# shellcheck source=./scope.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/scope.sh" 2>/dev/null || true
scope=""
if [ -n "$cwd" ] && command -v session_root >/dev/null 2>&1; then
  scope="$(session_root "$cwd")"
fi
search_args="$(jq -nc --arg q "$prompt" --argjson l "$limit" --arg c "$scope" '
  {query:$q, limit:$l, skip_if_warming:true, push:true}
  + (if ($c | length) > 0 then {scope_root: $c} else {} end)
')"
out="$(mcp_call memory_search "$search_args" "$to")"

[ -z "$out" ] && exit 0

hits="$(printf '%s' "$out" | jq -r --argjson k "$topk" --argjson min "$min_score" '
  (if type == "array" then . else [] end)
  | map(select((.hybrid_score // .score // 0) >= $min))
  | .[:$k]
  | .[]
  | "From memory (\(.type), from=\(.from // "derived"), \(.host // "unknown"), \((.created_at // "")[0:10]), score=\((.hybrid_score // .score // 0) * 100 | floor / 100), id=\(.id)): \(.content)"
' 2>/dev/null || true)"

# Heat (memory::heat): remember which rows this turn pushed, so the Stop hook
# (heat-stop.sh) can count them as recalled, and as used when the reply cites
# one. Claude Code only: Codex has no Stop hook with a transcript, so its
# pushes would never be matched with a use and must not be counted at all.
session_id="$(printf '%s' "$input" | jq -r '.session_id // empty' 2>/dev/null || true)"
pushed_file="${TMPDIR:-/tmp}/linggen-pushed-$(id -u)-${session_id//[^A-Za-z0-9_-]/}"
if [ -n "$session_id" ] && [ "${LINGGEN_CLIENT:-}" != "codex" ]; then
  printf '%s' "$out" | jq -r --argjson k "$topk" --argjson min "$min_score" '
    (if type == "array" then . else [] end)
    | map(select((.hybrid_score // .score // 0) >= $min)) | .[:$k] | .[] | select(.id) | "\(.id)\t\((.content // "")[0:30] | gsub("[\\t\\n]"; " "))"
  ' > "$pushed_file" 2>/dev/null || true
fi

hit_count="$(printf '%s\n' "$hits" | grep -c .)"
[ -n "$hits" ] && printf '%s\n' "$hits"

# Memory-upkeep nudge — undreamed days + open review items, from ONE
# `memory_upkeep` op=days call. Cached (default 5 min, so a solve or dream shows within minutes) so the recall path stays
# fast. Thresholds: ≥2 undreamed days (the
# engine's own 3am cron usually covers yesterday; nagging about one day
# is noise) and ≥1 open review item.
upkeep_ttl_min="${LINGGEN_UPKEEP_CACHE_MIN:-5}"
upkeep_cache="${TMPDIR:-/tmp}/linggen-upkeep-$(id -u)"
upkeep=""
if [ -f "$upkeep_cache" ] && [ -n "$(find "$upkeep_cache" -mmin "-$upkeep_ttl_min" 2>/dev/null)" ]; then
  upkeep="$(cat "$upkeep_cache" 2>/dev/null || true)"
else
  days_out="$(mcp_call memory_upkeep '{"op":"days","undreamed_only":true}' "$to")"
  if [ -n "$days_out" ]; then
    # dream_run is this machine's alone: another machine's host is offered
    # only the dream with its own model.
    upkeep="$(printf '%s' "$days_out" | jq -r --argjson here "${LINGGEN_LOCAL:-1}" '
      (.days // [] | length) as $p |
      (.open_issues // 0) as $i |
      ((.days // [])[0].date // "") as $o |
      [ (if $p >= 2 then "memory upkeep: \($p) days undreamed (oldest \($o)) — offer to run the dream: /linggen:dream runs it with this session'\''s model" + (if $here == 1 then "; memory_upkeep op=dream_run offloads it to the Linggen engine" else "" end) else empty end),
        (if $i >= 1 then "memory upkeep: \($i) item(s) awaiting review — offer /linggen:solve (memory_upkeep op=issues has the facts)" else empty end) ]
      | join("\n")
    ' 2>/dev/null || true)"
    printf '%s' "$upkeep" > "$upkeep_cache" 2>/dev/null || true
  fi
fi
[ -n "$upkeep" ] && printf '\n%s\n' "$upkeep"

# Always-on capture nudge — fires EVERY turn, including zero-hit turns
# (often the very turns that produce new memory). Tier definitions / routing
# live in the session-start MCP instructions; this is only the per-turn reminder.
cat <<'CAPTURE'

Memory capture: before finishing this turn, recognize anything worth remembering and write it at the right tier. Progress, status, what was built / fixed / tried / released → episodic. Semantic holds the user's own words (preferences, decisions with why), measured results and gotchas, each one conclusion + a file path, short (core/semantic adds are search-first); anchor relative time to absolute dates ("last month" → "2026-06"). Nothing worth keeping? Skip silently.
CAPTURE
# No source_session line: stamp-cwd.sh fills it on every add (host-filled,
# never asked of the model). Codex has no stamping seam yet, so its adds go
# unstamped until it ships PreToolUse input rewrite.

if [ "$hit_count" -ge 1 ]; then
  # THE wrong-memory footer is recall-note.txt, beside this script; the
  # engine's RECONCILE_FOOTER (core_block.rs) include_str!s the same file.
  # Fires on ANY hit: a single recalled row can be wrong too.
  printf '\n%s\n' "$(cat "$hooks_dir/recall-note.txt" 2>/dev/null)"
fi
