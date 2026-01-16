#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/get_library_pack.sh

if [ -f ".linggen/config.json" ]; then
    PROJECT_URL=$(jq -r '.api_url // empty' .linggen/config.json 2>/dev/null)
fi
API_URL=${LINGGEN_API_URL:-${PROJECT_URL:-"http://localhost:7000"}}

PACK_ID="$1"

if [ -z "$PACK_ID" ]; then
    echo "Usage: $0 <pack_id>"
    exit 1
fi

RESPONSE=$(curl -s -X GET "$API_URL/api/library/packs/$PACK_ID")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

# Check for error in response
if echo "$RESPONSE" | grep -q "Pack not found"; then
    echo "Error: Pack '$PACK_ID' not found."
    exit 1
fi

echo "$RESPONSE" | jq -r ".content // .error // .message // \"Error: Could not parse response\""
