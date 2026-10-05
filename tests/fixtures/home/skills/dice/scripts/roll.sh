#!/bin/sh
# A fixed roll (tests want the same answer every time): half the faces, rounded up.
faces="${1:-6}"
value=$(( (faces + 1) / 2 ))
dir="$(dirname "$0")/../data"
printf '{"guide":"Say the roll in one line.","table":{"name":"Harbor Table"},"player":{"name":"Alex"},"last_roll":{"faces":%s,"value":%s}}\n' "$faces" "$value" >"$dir/table.json"
cat "$dir/table.json"
