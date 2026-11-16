---
# The version/engine/browser line and the row counts come from the command's
# Bash fetch, which these runs don't grant; the upkeep and next lines come from
# memory_dream_status (mocked) and must render exactly.
type: regex
target: last_message
pattern: 'upkeep:.*11/12 scanned.*9/12 dreamed.*first unscanned 2026-10-04.*first undreamed 2026-10-02.*2 to solve'
---
