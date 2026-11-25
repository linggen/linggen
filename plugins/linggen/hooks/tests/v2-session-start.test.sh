#!/usr/bin/env bash
# Under v2 (`outside_agents = "v2"`) a host's session start prints the block
# the engine made (`memory_agents op=index`: Linggen's own notes by topic, then
# the other agents' lines) once, asks it for this host and this folder, and
# the per-turn hook (recall.sh) prints nothing and asks for nothing. Under v1
# the old recall is unchanged. A fake engine stands in for Linggen; it never
# touches a real one.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
autostart="$here/../autostart.sh"
recall="$here/../recall.sh"
tmp="$(mktemp -d)" || exit 1
trap 'kill "${fake:-0}" 2>/dev/null; rm -rf "$tmp"' EXIT
fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
command -v python3 >/dev/null || { printf 'skip: no python3\n'; exit 0; }
command -v jq >/dev/null || { printf 'skip: no jq\n'; exit 0; }

cat >"$tmp/fake.py" <<'PY'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
tmp = sys.argv[1]
BLOCK = "Below is what Linggen and the other agents remember.\nLinggen · the person's rules\n- Reply in Chinese (rule · language · id Zh00000001)\nCodex · the person, from ~/x\n- Keep replies short."
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{"ok":true}')
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))))
        name = req.get("params", {}).get("name"); a = req.get("params", {}).get("arguments", {})
        if name:
            with open(tmp + "/calls", "a") as f: f.write(json.dumps({"name": name, "args": a}, ensure_ascii=False) + "\n")
        if name == "memory_agents" and a.get("op") == "mode":
            out = {"outside_agents": open(tmp + "/mode").read().strip()}
        elif name == "memory_agents":
            out = {"outside_agents": "v2", "block": BLOCK, "lines": []}
        elif name == "memory_upkeep":
            out = {"block": "OLD CORE BLOCK"}
        else:
            out = [{"type": "fact", "created_at": "2026-11-10T12:00:00Z", "id": "old1", "content": "An old row."}]
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": json.dumps(out, ensure_ascii=False)}]}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json"); self.end_headers(); self.wfile.write(body)
srv = HTTPServer(("127.0.0.1", 0), H)
open(sys.argv[2], "w").write(str(srv.server_address[1]))
srv.serve_forever()
PY
python3 "$tmp/fake.py" "$tmp" "$tmp/port" &
fake=$!; disown "$fake" 2>/dev/null
for _ in $(seq 1 50); do [ -s "$tmp/port" ] && break; sleep 0.1; done
[ -s "$tmp/port" ] || fail "the fake engine did not start"

export HOME="$tmp/home" TMPDIR="$tmp/t" LINGGEN_NO_ENGINE_INSTALL=1
mkdir -p "$HOME" "$TMPDIR"
unset LINGGEN_HOST LINGGEN_TOKEN LINGGEN_URL LINGGEN_CLIENT
export LINGGEN_PORT="$(cat "$tmp/port")"
: >"$tmp/calls"

# The hook tells its host by where the plugin lives (~/.codex/ or ~/.claude/).
mkdir -p "$HOME/.codex/plugins" "$HOME/.claude/plugins"
ln -s "$(cd "$here/../.." && pwd)" "$HOME/.codex/plugins/linggen"
ln -s "$(cd "$here/../.." && pwd)" "$HOME/.claude/plugins/linggen"
start() { # codex | claude
  jq -nc --arg c "$tmp" '{session_id:"s1", hook_event_name:"SessionStart", cwd:$c}' \
    | CLAUDE_PLUGIN_ROOT="$HOME/.$1/plugins/linggen" bash "$autostart" 2>/dev/null
}
turn() {
  jq -nc --arg p "$1" --arg c "$tmp" '{session_id:"s1", hook_event_name:"UserPromptSubmit", prompt:$p, cwd:$c}' | bash "$recall" 2>/dev/null
}

echo v2 >"$tmp/mode"
out="$(start codex)"
ctx="$(printf '%s' "$out" | jq -r '.hookSpecificOutput.additionalContext // empty')"
printf '%s' "$ctx" | grep -qF 'Reply in Chinese (rule · language · id Zh00000001)' || fail "v2 start did not print the engine's block: $out"
printf '%s' "$ctx" | grep -qF 'OLD CORE BLOCK' && fail "v2 start printed the v1 core block"
grep -q '"name": "memory_agents".*"op": "index"' "$tmp/calls" || fail "v2 start did not ask memory_agents op=index"
grep -q '"host": "codex"' "$tmp/calls" || fail "the block was not asked for this host"
grep -q '"cwd": ' "$tmp/calls" || fail "the block was not asked for this folder"
grep -q '"name": "memory_upkeep"' "$tmp/calls" && fail "v2 start asked the v1 store"

# Per turn: nothing is injected, and nothing is asked.
: >"$tmp/calls"
out="$(turn 'what does the starter plan include for live lessons? tell me everything')"
[ -z "$out" ] || fail "v2 printed something per turn: $out"
grep -q '"name": "memory_search"' "$tmp/calls" && fail "v2 recall searched the v1 store"
grep -q '"name": "memory_notes"' "$tmp/calls" && fail "v2 asked for note titles per turn"

# v1: the old core block and the old recall, unchanged.
echo v1 >"$tmp/mode"
: >"$tmp/calls"
out="$(start claude)"
printf '%s' "$out" | jq -r '.hookSpecificOutput.additionalContext // empty' | grep -qF 'OLD CORE BLOCK' || fail "v1 lost its core block: $out"
out="$(turn 'what does the starter plan include for live lessons? tell me everything')"
printf '%s' "$out" | grep -q '^From memory' || fail "v1 lost its recall rows: $out"
printf 'ok: under v2 a session start prints the engine block once and a turn prints nothing; v1 is unchanged\n'
