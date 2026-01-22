#!/bin/bash

# Default Linggen API URL
API_URL=${LINGGEN_API_URL:-"http://localhost:8787"}

# Try to load workspace-level override if it exists.
# The scripts are typically installed in .claude/skills/linggen/scripts/
# so the workspace root is 4 levels up.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../" && pwd)"

if [ -f "$WORKSPACE_ROOT/.linggen/config" ]; then
    # The config file should be a simple shell script or env file, e.g.:
    # LINGGEN_API_URL="http://192.168.1.100:7000"
    source "$WORKSPACE_ROOT/.linggen/config"
    API_URL=${LINGGEN_API_URL:-$API_URL}
fi

export API_URL
