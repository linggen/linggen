#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/get_status.sh

if [ -f ".linggen/config.json" ]; then
    PROJECT_URL=$(jq -r '.api_url // empty' .linggen/config.json 2>/dev/null)
fi
API_URL=${LINGGEN_API_URL:-${PROJECT_URL:-"http://localhost:7000"}}

RESPONSE=$(curl -s -X GET "$API_URL/api/status")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Linggen Status"
echo ""
echo "$RESPONSE" | jq -r '"**Status:** \(.status)\n**Message:** \(.message // "N/A")\n**Progress:** \(.progress // "N/A")"'
