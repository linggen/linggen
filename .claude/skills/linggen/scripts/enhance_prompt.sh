#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/enhance_prompt.sh
source "$(dirname "$0")/config.sh"
QUERY="$1"
STRATEGY="${2:-full_code}"
SOURCE_ID="$3"

if [ -z "$QUERY" ]; then
    echo "Usage: $0 <query> [strategy] [source_id]"
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

echo "$RESPONSE" | jq -r '"## Enhanced Prompt\n\n**Detected Intent:** \(.intent)\n\n---\n\n\(.enhanced_prompt)\n\n---\n\n### Context Sources\n" + ([.context_metadata[] | "- `\(.file_path)` (\(.source_id))"] | join("\n"))'
