#!/usr/bin/env bash
# Under v2 the per-turn hook (recall.sh) asks the engine for the TITLES of
# Linggen's notes the prompt's words match (memory_notes op=titles), prints
# "相关笔记: <summary> (id)", repeats none in one session, prints nothing when
# nothing matches or no engine answers — and under v1 it is the old recall.
# A fake engine stands in for Linggen; it never touches a real one.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
hook="$here/../recall.sh"
tmp="$(mktemp -d)" || exit 1
trap 'kill "${fake:-0}" 2>/dev/null; rm -rf "$tmp"' EXIT
fail() { printf 'FAIL %s\n' "$1" >&2; exit 1; }
command -v python3 >/dev/null || { printf 'skip: no python3\n'; exit 0; }

# The fake door: op=mode answers $MODE_FILE's word; op=titles lists the
# notes (id TAB summary) of $NOTES whose summary holds a word of the prompt,
# leaving out `exclude`; memory_search (the v1 recall) answers one row.
cat >"$tmp/notes" <<'NOTES'
Abc1234567	The starter plan cannot include live lessons
ZhCn000001	回复要用中文，代码和提交用英文
NOTES
cat >"$tmp/fake.py" <<'PY'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
tmp = sys.argv[1]
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{"ok":true}')
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))))
        name = req["params"]["name"]; a = req["params"]["arguments"]
        with open(tmp + "/calls", "a") as f: f.write(json.dumps({"name": name, "args": a}, ensure_ascii=False) + "\n")
        if name == "memory_agents":
            out = {"outside_agents": open(tmp + "/mode").read().strip()}
        elif name == "memory_notes":
            words = a["words"].lower().replace("?", " ").split()
            hits = []
            for line in open(tmp + "/notes", encoding="utf-8").read().splitlines():
                nid, summary = line.split("\t")
                hay = summary.lower()
                if nid not in a.get("exclude", []) and any(w in hay for w in words if len(w) > 2):
                    hits.append({"id": nid, "summary": summary, "kind": "decision", "topic": "t"})
            out = {"titles": hits[: a.get("limit", 5)]}
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

export HOME="$tmp/home" TMPDIR="$tmp/t"
mkdir -p "$HOME" "$TMPDIR"
unset LINGGEN_HOST LINGGEN_TOKEN LINGGEN_URL
export LINGGEN_PORT="$(cat "$tmp/port")"
: >"$tmp/calls"

# The hook as a host runs it: the prompt's JSON on stdin.
run() { # prompt [session]
  jq -nc --arg p "$1" --arg s "${2:-s1}" '{session_id:$s, hook_event_name:"UserPromptSubmit", prompt:$p, cwd:"/work/app"}' | bash "$hook" 2>/dev/null
}

echo v2 >"$tmp/mode"

out="$(run 'what does the pricing plan include? the starter plan')"
[ "$out" = '相关笔记: The starter plan cannot include live lessons (Abc1234567)' ] || fail "an English prompt got: $out"

out="$(run '回复要用中文')"
printf '%s' "$out" | grep -qF '相关笔记: 回复要用中文，代码和提交用英文 (ZhCn000001)' || fail "a Chinese prompt got: $out"

# No repeat within a session; another session is shown it again.
[ -z "$(run 'the starter plan again')" ] || fail "a title was repeated in one session"
[ -n "$(run 'the starter plan again' s2)" ] || fail "a second session was not shown the title"
# The engine was told which ids to leave out.
grep -q '"exclude": \["Abc1234567"\]' "$tmp/calls" || grep -q '"exclude": \[.*Abc1234567' "$tmp/calls" || fail "exclude not sent"

# Nothing matches: nothing printed, no capture reminder, no recall rows.
out="$(run 'please rename this function' s3)"
[ -z "$out" ] || fail "a prompt about nothing printed: $out"
# A one-character prompt does not even ask.
: >"$tmp/calls"
[ -z "$(run 'x' s4)" ] && [ ! -s "$tmp/calls" ] || fail "a one-character prompt asked the engine"

# No engine: silent, exit 0, and fast.
real_port="$LINGGEN_PORT"
export LINGGEN_PORT=1
start=$SECONDS
out="$(jq -nc '{session_id:"s5", prompt:"the starter plan", cwd:"/x"}' | bash "$hook" 2>&1)"; code=$?
[ "$code" = 0 ] && [ -z "$out" ] || fail "no engine: exit $code, output: $out"
[ $((SECONDS - start)) -le 2 ] || fail "no engine took too long"
export LINGGEN_PORT="$real_port"

# v1: the old recall, unchanged — rows from memory_search, the capture reminder, no titles.
echo v1 >"$tmp/mode"
out="$(run 'what does the pricing plan include? the starter plan' s6)"
printf '%s' "$out" | grep -q '^From memory' || fail "v1 lost its recall rows: $out"
printf '%s' "$out" | grep -q 'Memory capture:' || fail "v1 lost its capture reminder"
printf '%s' "$out" | grep -q '相关笔记' && fail "v1 printed titles"
: >"$tmp/calls"
[ -z "$(run 'short' s7)" ] || fail "v1 short prompt printed something"
printf 'ok: under v2 a turn gets matching note titles once, quietly; v1 recall is unchanged\n'
