#!/usr/bin/env bash
set -u
here="$(cd "$(dirname "$0")" && pwd)"
hook="$here/../recall-on-delegate.sh"
tmp="$(mktemp -d)" || exit 1
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/search" <<'FAKE'
#!/usr/bin/env bash
case "${FAKE_MODE:-hits}" in
  fail) exit 1 ;;
  empty) echo '[]' ;;
  invalid) echo 'not json' ;;
  *) printf '%s\n' '[{"type":"fact","created_at":"2026-11-10T12:00:00Z","id":"abc123","content":"A useful memory."}]' ;;
esac
FAKE
chmod +x "$tmp/search"
export LINGGEN_SEARCH_CMD="$tmp/search"
unset LINGGEN_DELEGATE
prompt='Please investigate this substantial task and report back with a detailed result.'
payload() { jq -nc --arg t "$1" --arg p "${2:-$prompt}" '{tool_name:$t,tool_input:{prompt:$p,description:"Investigate",subagent_type:"general",model:"test"},cwd:"/tmp",session_id:"s"}'; }
run() { printf '%s' "$1" | bash "$hook" 2>&1; }
fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
silent() { local out; out="$(run "$1")" && [ -z "$out" ]; }
for tool in Agent Task; do
  input="$(payload "$tool")"
  out="$(LINGGEN_DELEGATE_REWRITE=1 LINGGEN_DELEGATE_DECISION=allow run "$input")" || fail "$tool exit"
  printf '%s' "$out" | jq -e '.hookSpecificOutput | .hookEventName == "PreToolUse" and (.additionalContext | contains("id=abc123")) and (keys == ["additionalContext", "hookEventName"])' >/dev/null || fail "$tool context only"
done
for tool in mcp__plugin_linggen_linggen__agent_run mcp__linggen__agent_run collaborationspawn_agent; do
  silent "$(payload "$tool")" || fail "$tool must be silent"
done
for mode in empty fail invalid; do
  FAKE_MODE="$mode" silent "$input" || fail "$mode must be silent"
done
silent "$(payload Agent short)" || fail 'short prompt'
silent 'not JSON' || fail 'malformed input'
silent "$(payload Agent "$prompt (id=abc123)")" || fail 'duplicate row'
LINGGEN_DELEGATE=0 silent "$input" || fail 'disabled hook'

# The separate memory attribution hook still supplies host metadata.
stamp="$here/../stamp-cwd.sh"
add='{"tool_name":"mcp__memory__memory_add","tool_input":{"content":"x"},"cwd":"/tmp","session_id":"s1"}'
printf '%s' "$add" | LINGGEN_CLIENT=codex bash "$stamp" | jq -e '.hookSpecificOutput.updatedInput | .host == "codex" and .source_session == "s1"' >/dev/null || fail 'Codex stamp'
printf '%s' "$add" | bash "$stamp" | jq -e '.hookSpecificOutput.updatedInput.host == "claude-code"' >/dev/null || fail 'Claude Code stamp'
printf 'ok: delegation hooks leave tool input and permission untouched\n'
