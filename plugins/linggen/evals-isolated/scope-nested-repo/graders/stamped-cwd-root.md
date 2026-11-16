---
# stamp-cwd.sh adds cwd and root = the session's workspace (no git above it).
type: regex
target: trace
pattern: '\\"cwd\\":\\"[^\\"]*/cwd\\",\\"root\\":\\"[^\\"]*/cwd\\"'
---
