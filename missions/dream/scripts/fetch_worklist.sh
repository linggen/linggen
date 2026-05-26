#!/usr/bin/env bash
# Dream mission entry script — pre-fetch the past-TTL episodic worklist.
#
# The scheduler reads this script's stdout to decide whether to even
# spawn the LLM:
#   - First line `WORKLIST_SIZE=0`   → no work; scheduler emits the
#     final `CONSOLIDATED promoted=0 deleted=0` status and skips the
#     agent loop entirely (avoids the gpt-5.5 empty-result loop).
#   - First line `WORKLIST_SIZE=<n>` (n>0) → write the row JSON to
#     `$MISSION_OUTPUT_DIR/worklist.json` so the agent can pick it up,
#     then return; the LLM does the per-row judgment work.
#
# Exit 0 = ran cleanly (work-or-no-work).
# Exit non-zero = unrecoverable error (daemon unreachable, malformed
# response). The scheduler treats non-zero as a failed mission run.

set -euo pipefail

# Resolve ling-mem URL from the daemon config if present, else default.
LING_MEM_URL="${LING_MEM_URL:-http://127.0.0.1:9888}"

# Past-TTL list. `past_ttl: true` tells the daemon to resolve the
# cutoff server-side from its configured `episodic_ttl_days`. No
# `type`/`from`/`outcome` filters — we want every past-TTL episodic
# row regardless of category.
RESPONSE="$(
  curl -fsS -X POST \
    -H 'Content-Type: application/json' \
    -d '{"episodic": true, "past_ttl": true, "limit": 200, "sort": "oldest"}' \
    "${LING_MEM_URL}/api/memory/list"
)" || {
  echo "ERROR: ling-mem list failed (curl exit $?, url=${LING_MEM_URL})" >&2
  exit 1
}

# Envelope shape: {"ok": true, "data": [...]} on success.
OK="$(printf '%s' "$RESPONSE" | jq -r '.ok // false')"
if [ "$OK" != "true" ]; then
  ERR="$(printf '%s' "$RESPONSE" | jq -r '.error // "unknown"')"
  echo "ERROR: ling-mem returned ok=false: ${ERR}" >&2
  exit 2
fi

ROWS="$(printf '%s' "$RESPONSE" | jq -c '.data')"
COUNT="$(printf '%s' "$ROWS" | jq 'length')"

# Always write the worklist file so the agent can read it on the
# non-empty path. For COUNT=0 the file is `[]` — also valid JSON.
printf '%s' "$ROWS" > "${MISSION_OUTPUT_DIR}/worklist.json"

# Single-line marker the scheduler greps for. Keep this format stable.
echo "WORKLIST_SIZE=${COUNT}"
