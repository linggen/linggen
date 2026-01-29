#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/get_library_pack.sh
source "$(dirname "$0")/config.sh"
PACK_ID="$1"

if [ -z "$PACK_ID" ]; then
    echo "Usage: $0 <pack_id>"
    exit 1
fi

ENC_PACK_ID=$(python3 - <<'PY'
import sys, urllib.parse
print(urllib.parse.quote(sys.argv[1], safe=""))
PY
"$PACK_ID")

RESPONSE=$(curl -s -X GET "$API_URL/api/library/packs/$ENC_PACK_ID")

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
