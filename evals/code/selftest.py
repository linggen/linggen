#!/usr/bin/env python3
"""Exercise the LIVE code paths of run.py against stand-ins, with no real engine
and no real model.

  * a fake engine (HTTP + WebSocket on a free local port - never 9527/9528)
    that speaks /mcp and the wait socket the way doc/mcp-spec.md and
    src/server/mcp_agent.rs describe them. It "does the work" by applying a
    task's canned change: the good one for work "code", the bad one for
    work "none"; one run raises a permission request and waits for a deny;
    one run finishes inside the first call;
  * a fake `claude` executable that prints stream-json like `claude -p
    --output-format stream-json --verbose` and applies the canned good change.

What this proves is the runner's client code against MY READING of the docs,
not against the real engine or the real claude.

  /usr/bin/python3 evals/code/selftest.py
"""

import base64
import hashlib
import json
import os
import re
import shutil
import stat
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check  # noqa: E402
import run  # noqa: E402

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
CELL_NAME = re.compile(r"^(?P<task>.+)-(?P<arm>code|plain|claude)-[^-]+$")
ASK_CELL = ("py-ship-trap", "none")     # (task, work): this run asks for permission once
DIRECT_CELL = ("js-boundary-test", "code")  # this run finishes inside agent_run


class Fake:
    sessions = {}
    lock = threading.Lock()
    n = 0


def apply_change(work, write_dir):
    cell = CELL_NAME.match(Path(write_dir).name)
    task = check.load_task(cell["task"])
    arm = "dry-good" if work == "code" else "dry-bad"
    out = run.run_dry(arm, task, Path(write_dir))
    return (cell["task"], work), out["reply"]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method, params = body["method"], body.get("params", {})
        if method == "initialize":
            result = {"capabilities": {"tools": {}}}
        elif method == "tools/list":
            props = {k: {} for k in ("prompt", "work", "write_dir", "model", "grants", "permission_mode")}
            result = {"tools": [{"name": "agent_run", "inputSchema": {"properties": props}},
                                {"name": "agent_status"}, {"name": "agent_permit"}]}
        else:
            result = self.tool(params["name"], params.get("arguments", {}))
        data = json.dumps({"jsonrpc": "2.0", "id": body["id"], "result": result}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)

    def tool(self, name, args):
        def text(s, err=False):
            return {"content": [{"type": "text", "text": s}], "isError": err}
        if name == "agent_models":
            return text("fake-model (default)")
        if name == "agent_run":
            assert args["permission_mode"] == "acceptEdits" and args["model_fallback"] is False
            assert args["grants"]["tools"], "tools grant missing"
            cell, reply = apply_change(args["work"], args["write_dir"])
            if cell == DIRECT_CELL:
                return text(reply + '\n\nPreset grants: {"granted":{"tools":["python3"]},"refused":[]}')
            with Fake.lock:
                Fake.n += 1
                sid = "sess-%d-fake" % Fake.n
                Fake.sessions[sid] = {"reply": reply, "done_at": time.time() + 0.8, "cell": cell,
                                      "asks": [{"id": "req-1", "tool": "Bash", "summary": "sh ship.sh",
                                                "level": "edit", "path": "/x"}] if cell == ASK_CELL else [],
                                      "denied": []}
            return text("Still running in the background in Linggen session %s. Don't poll: watch ws://x" % sid)
        s = Fake.sessions.get(args.get("session_id"))
        if name == "agent_permit":
            assert args["decision"] == "deny"
            s["denied"].append(args["request_id"])
            return text(json.dumps({"request_id": args["request_id"], "decision": "deny"}))
        if name == "agent_status":
            return text(json.dumps(status(s)))
        if name in ("agent_session_close", "agent_stop"):
            return text("stopped")
        return text("unknown tool " + name, True)

    def do_GET(self):
        sid = self.path.split("session_id=")[1]
        s = Fake.sessions[sid]
        key = self.headers["Sec-WebSocket-Key"]
        accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
        self.wfile.write(("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                          "Sec-WebSocket-Accept: %s\r\n\r\n" % accept).encode())
        sent = False
        while True:
            st = status(s)
            if st["pending"] and not sent:
                sent = True
                self.frame({"type": "permission_request", "session_id": sid, "request": st["pending"][0]})
            if not st["running"]:
                self.frame({"type": "finished", "session_id": sid, "status": "completed",
                            "reply": s["reply"], "denied": [], "error": None, "model": "fake-model"})
                self.wfile.write(b"\x88\x00")
                return
            time.sleep(0.1)

    def frame(self, obj):
        payload = json.dumps(obj).encode()
        head = bytes([0x81, len(payload)]) if len(payload) < 126 else bytes([0x81, 126]) + len(payload).to_bytes(2, "big")
        self.wfile.write(head + payload)
        self.wfile.flush()


def status(s):
    pending = [a for a in s["asks"] if a["id"] not in s["denied"]]
    running = bool(pending) or time.time() < s["done_at"]
    return {"running": running, "pending": pending, "status": None if running else "completed",
            "reply": s["reply"], "denied": [], "error": None, "model": "fake-model"}


FAKE_CLAUDE = '''#!/usr/bin/env python3
import json, os, sys
sys.path.insert(0, {here!r})
import check, run
from pathlib import Path
import re
sys.stdin.read()
work = Path(os.getcwd())
cell = re.match(r"^(?P<task>.+)-claude-[^-]+$", work.name)
out = run.run_dry("dry-good", check.load_task(cell["task"]), work)
for i in range(3):
    print(json.dumps({{"type": "assistant", "message": {{"content": [{{"type": "tool_use", "name": "Read"}}]}}}}))
print(json.dumps({{"type": "result", "subtype": "success", "is_error": False, "result": out["reply"],
                  "num_turns": 4, "permission_denials": [{{"tool_name": "Bash"}}]}}))
'''


def main():
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    port = server.server_address[1]
    assert port not in (9527, 9528)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    tmp = Path(tempfile.mkdtemp(prefix="codeeval-selftest-"))
    try:
        fake_claude = tmp / "claude"
        fake_claude.write_text(FAKE_CLAUDE.format(here=str(Path(__file__).resolve().parent)))
        fake_claude.chmod(fake_claude.stat().st_mode | stat.S_IXUSR)
        out_dir = tmp / "results"
        run.main(["--yes", "--engine", "http://127.0.0.1:%d" % port, "--model", "fake-model",
                  "--claude-bin", str(fake_claude), "--timeout", "60", "--out", str(out_dir),
                  "--scratch", str(tmp)])
        data = json.loads((out_dir / "results.json").read_text())
        cells = {(r["task"], r["arm"]): r for r in data["records"]}
        problems = []
        for (task, arm), r in cells.items():
            want = arm in ("code", "claude")
            if r["passed"] != want:
                problems.append("%s/%s passed=%s want %s (%s)" % (task, arm, r["passed"], want, r["failed"]))
        ask = cells[("py-ship-trap", "plain")]
        if ask["asks"] != 1:
            problems.append("permission ask not counted: %r" % ask["asks"])
        if cells[("js-boundary-test", "code")]["asks"] != 0 or cells[("js-boundary-test", "code")]["session"] is not None:
            problems.append("direct-return run not handled")
        if any(cells[(t, "claude")]["tool_calls"] != 3 or cells[(t, "claude")]["asks"] != 1
               for t in {k[0] for k in cells}):
            problems.append("claude tool calls / denials not read from stream-json")
        if "Preset grants" in (out_dir / DIRECT_CELL[0] / "code" / "run1" / "reply.txt").read_text():
            problems.append("grants line left in the reply")
        print("\nselftest: %d cells against the fake engine and fake claude" % len(cells))
        if problems:
            print("PROBLEMS:\n  " + "\n  ".join(problems))
            sys.exit(1)
        print("selftest ok: code and claude arms pass, plain arm fails (bad change), ask answered 'deny' and counted")
    finally:
        server.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
