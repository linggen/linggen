#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/query_codebase.sh
source "$(dirname "$0")/config.sh"
QUERY="$1"
LIMIT="${2:-3}"
EXCLUDE_SOURCE_ID="$3"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [limit] [exclude_source_id]"
    exit 1
fi

DATA=$(jq -n \
  --arg query "$QUERY" \
  --argjson limit "$LIMIT" \
  --arg exclude_source_id "$EXCLUDE_SOURCE_ID" \
  '{query: $query, limit: $limit, exclude_source_id: (if $exclude_source_id == "" then null else $exclude_source_id end)} | with_entries(select(.value != null))')

RESPONSE=$(curl -s -X POST "$API_URL/api/query" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "$RESPONSE" | jq -r ".results[:$LIMIT] | to_entries | .[] | \"--- Chunk \((.key + 1)) [\(.value.source_id)] ---\nFile: \(.value.document_id)\n\n\(.value.content)\n\""
