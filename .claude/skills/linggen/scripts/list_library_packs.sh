#!/bin/bash
# backend/api/library_templates/skills/linggen/scripts/list_library_packs.sh
source "$(dirname "$0")/config.sh"

RESPONSE=$(curl -s -X GET "$API_URL/api/library")

if [ $? -ne 0 ]; then
    echo "Error: Could not connect to linggen-server at $API_URL"
    exit 1
fi

echo "## Global Library Packs"
echo ""
echo "$RESPONSE" | jq -r '.packs[] | "### \(.name)\n- **ID:** `\(.id)`\n- **Folder:** \(.folder // "root")\n- **Description:** \(.description // "No description")\n"'
