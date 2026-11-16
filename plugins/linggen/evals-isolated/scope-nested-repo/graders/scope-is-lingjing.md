---
# The memory_add mock echoes the input it received — AFTER stamp-cwd.sh — so
# the tool result in the trace (JSON-escaped) shows what the daemon would get.
# The named scope must survive and point at the real repo: relative
# `lingjing` (resolved against root) or the absolute <workspace>/lingjing.
type: regex
target: trace
pattern: '\\"scope\\":\\"(lingjing|[^\\"]*/cwd/lingjing)/?\\"'
---
