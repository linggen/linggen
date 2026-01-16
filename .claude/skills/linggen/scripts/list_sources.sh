#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/list_sources.sh

if [ -f ".linggen/config.json" ]; then
    PROJECT_URL=$(jq -r '.api_url // empty' .linggen/config.json 2>/dev/null)
fi
API_URL=${LINGGEN_API_URL:-${PROJECT_URL:-"http://localhost:7000"}}

RESPONSE=$(curl -s -X GET "$API_URL/api/resources")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

COUNT=$(echo "$RESPONSE" | jq '. | length')
echo "## Indexed Sources ($COUNT total)"
echo ""
echo "$RESPONSE" | jq -r '.[] | "### \(.name)\n- **ID:** `\(.id)`\n- **Type:** \(.source_type)\n- **Path:** `\(.path)`\n- **Files:** \(.file_count // 0)\n- **Chunks:** \(.chunk_count // 0)\n"'
