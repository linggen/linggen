#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/search_codebase.sh
source "$(dirname "$0")/config.sh"
QUERY="$1"
STRATEGY="${2:-full_code}"
LIMIT="${3:-5}"
SOURCE_ID="$4"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [strategy] [limit] [source_id]"
    exit 1
fi

DATA=$(jq -n \
  --arg query "$QUERY" \
  --arg strategy "$STRATEGY" \
  --arg source_id "$SOURCE_ID" \
  '{query: $query, strategy: $strategy, source_id: (if $source_id == "" then null else $source_id end)} | with_entries(select(.value != null))')

RESPONSE=$(curl -s -X POST "$API_URL/api/enhance" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Search Results for: $QUERY"
echo "$RESPONSE" | jq -r ".context_chunks[:$LIMIT] | to_entries | .[] | \"--- Chunk \((.key + 1)) ---\n\(.value)\n\""
