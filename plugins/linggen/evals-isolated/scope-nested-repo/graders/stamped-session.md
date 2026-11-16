---
# stamp-cwd.sh adds this session's id and the host.
type: regex
target: trace
pattern: '\\"source_session\\":\\"[0-9a-f-]{36}\\",\\"host\\":\\"claude-code\\"'
---
