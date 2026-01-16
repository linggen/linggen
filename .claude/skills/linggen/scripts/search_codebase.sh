#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/search_codebase.sh

# 1. Try to find a project-level config for the URL
if [ -f ".linggen/config.json" ]; then
    PROJECT_URL=$(jq -r '.api_url // empty' .linggen/config.json 2>/dev/null)
fi

# 2. Set API_URL using the hierarchy: Env > Config File > Default
API_URL=${LINGGEN_API_URL:-${PROJECT_URL:-"http://localhost:7000"}}

QUERY="$1"
STRATEGY="${2:-full_code}"
LIMIT="${3:-5}"
SOURCE_ID="$4"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [strategy] [limit] [source_id]"
    exit 1
fi

DATA=$(cat <<EOF
{
  "query": "$QUERY",
  "strategy": "$STRATEGY",
  "source_id": ${SOURCE_ID:+"$SOURCE_ID"}
}
EOF
)

DATA=$(echo "$DATA" | jq 'with_entries(select(.value != null))')

RESPONSE=$(curl -s -X POST "$API_URL/api/enhance" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Search Results for: $QUERY"
echo "$RESPONSE" | jq -r ".context_chunks[:$LIMIT] | to_entries | .[] | \"--- Chunk \((.key + 1)) ---\n\(.value)\n\""
