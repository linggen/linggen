#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/list_sources.sh
source "$(dirname "$0")/config.sh"

RESPONSE=$(curl -s -X GET "$API_URL/api/resources")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

COUNT=$(echo "$RESPONSE" | jq '.resources | length')
echo "## Indexed Sources ($COUNT total)"
echo ""
echo "$RESPONSE" | jq -r '.resources[] | "### \(.name)\n- **ID:** `\(.id)`\n- **Type:** \(.resource_type)\n- **Path:** `\(.path)`\n- **Files:** \(.stats.file_count // 0)\n- **Chunks:** \(.stats.chunk_count // 0)\n"'
