#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/memory_search_semantic.sh

if [ -f ".linggen/config.json" ]; then
    PROJECT_URL=$(jq -r '.api_url // empty' .linggen/config.json 2>/dev/null)
fi
API_URL=${LINGGEN_API_URL:-${PROJECT_URL:-"http://localhost:7000"}}

QUERY="$1"
LIMIT="${2:-10}"
SOURCE_ID="$3"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [limit] [source_id]"
    exit 1
fi

DATA=$(cat <<EOF
{
  "query": "$QUERY",
  "limit": $LIMIT,
  "source_id": ${SOURCE_ID:+"$SOURCE_ID"}
}
EOF
)

DATA=$(echo "$DATA" | jq 'with_entries(select(.value != null))')

RESPONSE=$(curl -s -X POST "$API_URL/api/memory/search_semantic" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Semantic Memory Results for: $QUERY"
echo ""
echo "$RESPONSE" | jq -r ".results[:$LIMIT] | .[] | \"### \(.title // "Untitled") [\(.source_id)]\n- File: `\(.file_path)`\n- Snippet: \(.snippet)...\n\""
