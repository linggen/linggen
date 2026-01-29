#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/memory_fetch_by_meta.sh
source "$(dirname "$0")/config.sh"
KEY="$1"
VALUE="$2"

if [ -z "$KEY" ] || [ -z "$VALUE" ]; then
    echo "Usage: $0 <key> <value>"
    exit 1
fi

DATA=$(cat <<EOF
{
  "key": "$KEY",
  "value": "$VALUE"
}
EOF
)

RESPONSE=$(curl -s -X POST "$API_URL/api/memory/fetch_by_meta" \
  -H "Content-Type: application/json" \
  -d "$DATA")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

# Check for error in response
if echo "$RESPONSE" | grep -q "\"error\""; then
    echo "Error: $(echo "$RESPONSE" | jq -r '.error // .')"
    exit 1
fi

echo "$RESPONSE" | jq -r '"## Memory: \(.title)\nTags: \(.tags | join(", "))\n\n---\n\n\(.body)"'
