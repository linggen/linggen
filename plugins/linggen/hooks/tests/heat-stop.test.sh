#!/usr/bin/env bash
# recall.sh remembers the rows it pushed; heat-stop.sh bumps recalled for each
# and used for those the reply cites. A fake MCP door (python) records calls.
# The wrong-memory footer (recall-note.txt) is what recall.sh prints.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
tmp="$(mktemp -d "$here/.tmp.XXXXXX")" || exit 1
trap 'kill "${srv:-0}" 2>/dev/null; rm -rf "$tmp"' EXIT
export HOME="$tmp/home" TMPDIR="$tmp"; mkdir -p "$HOME"
export LINGGEN_DATA_DIR="$tmp/.linggen" LINGGEN_RECALL_DISABLE=0
fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
command -v python3 >/dev/null || { echo "skip: no python3"; exit 0; }

cat >"$tmp/door.py" <<'PY'
import http.server, json, sys
log = open(sys.argv[1], "a")
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        p = body["params"]
        if p["name"] == "memory_search" and "query" in p["arguments"]:
            text = json.dumps([{"id": "AAA1", "content": "bees row", "type": "fact", "score": 0.9},
                               {"id": "BBB2", "content": "goats row", "type": "fact", "score": 0.8}])
        else:
            text = "{}"
        log.write(json.dumps(p) + "\n"); log.flush()
        out = json.dumps({"jsonrpc": "2.0", "id": 1, "result": {"content": [{"type": "text", "text": text}]}}).encode()
        self.send_response(200); self.send_header("Content-Length", str(len(out))); self.end_headers(); self.wfile.write(out)
s = http.server.HTTPServer(("127.0.0.1", 0), H)
print(s.server_port, flush=True)
s.serve_forever()
PY
python3 "$tmp/door.py" "$tmp/calls" >"$tmp/port" & srv=$!
for _ in 1 2 3 4 5 6 7 8 9 10; do [ -s "$tmp/port" ] && break; sleep 0.3; done
export LINGGEN_HOST=127.0.0.1 LINGGEN_PORT="$(cat "$tmp/port")"

sid=s-heat
out="$(jq -nc --arg s "$sid" '{prompt:"tell me about the bees please", cwd:"/nonexistent", session_id:$s}' | bash "$here/../recall.sh")"
note="$(cat "$here/../recall-note.txt")"
case "$out" in *"$note"*) ;; *) fail "recall.sh did not print recall-note.txt" ;; esac
[ "${#note}" -le 450 ] || fail "the footer is over 450 chars"
[ -s "$tmp/linggen-pushed-$(id -u)-$sid" ] || fail "no pushed-ids file"

# A transcript whose last reply cites AAA1 only.
cat >"$tmp/t.jsonl" <<'J'
{"type":"user","message":{"role":"user","content":"tell me about the bees please"}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"From memory (2026-11-01, id=AAA1): bees row. Yes."}]}}
J
: >"$tmp/calls"
jq -nc --arg s "$sid" --arg t "$tmp/t.jsonl" '{session_id:$s, transcript_path:$t}' | bash "$here/../heat-stop.sh"
a="$(grep '"AAA1"' "$tmp/calls" | grep memory_update)"
b="$(grep '"BBB2"' "$tmp/calls" | grep memory_update)"
case "$a" in *'"bump_used": 1'*) ;; *) fail "AAA1 should count as used: $a" ;; esac
case "$b" in *'"bump_used": 0'*) ;; *) fail "BBB2 should not count as used: $b" ;; esac
case "$b" in *'"bump_recalled": 1'*) ;; *) fail "BBB2 should count as recalled: $b" ;; esac
[ ! -e "$tmp/linggen-pushed-$(id -u)-$sid" ] || fail "pushed file should be consumed"

# Codex: no heat at all.
export LINGGEN_CLIENT=codex
jq -nc '{prompt:"tell me about the bees please", cwd:"/nonexistent", session_id:"s-codex"}' | bash "$here/../recall.sh" >/dev/null
[ ! -e "$tmp/linggen-pushed-$(id -u)-s-codex" ] || fail "codex must not record pushes"
unset LINGGEN_CLIENT

# No door, no transcript: silent, exit 0.
jq -nc '{session_id:"nope"}' | LINGGEN_PORT=1 bash "$here/../heat-stop.sh" || fail "must exit 0 on failure"
echo "ok heat-stop"
