#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/memory_search_semantic.sh
source "$(dirname "$0")/config.sh"
QUERY="$1"
LIMIT="${2:-10}"
SOURCE_ID="$3"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [limit] [source_id]"
    exit 1
fi

DATA=$(jq -n \
  --arg query "$QUERY" \
  --argjson limit "$LIMIT" \
  --arg source_id "$SOURCE_ID" \
  '{query: $query, limit: $limit, source_id: (if $source_id == "" then null else $source_id end)} | with_entries(select(.value != null))')

RESPONSE=$(curl -s -X POST "$API_URL/api/memory/search_semantic" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Semantic Memory Results for: $QUERY"
echo ""
echo "$RESPONSE" | jq -r --arg limit "$LIMIT" '.results[:($limit|tonumber)] | .[] | "### \(.title // "Untitled") [\(.source_id)]\n- File: `\(.file_path)`\n- Snippet: \(.snippet)...\n"'
