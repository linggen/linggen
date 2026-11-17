#!/usr/bin/env python3
"""Code eval runner: does the `code` work section make a Linggen session better?

Every task runs three ways, each in a fresh copy of the task's project:

  code    a Linggen delegated run over the engine's /mcp, work "code"
  plain   the same run with work "none" (same model)
  claude  a headless Claude Code session (`claude -p`), cwd = the copy

then the task's hidden check scores the copy (check.py) and the table is
written to results/<timestamp>/ as table.md and results.json.

THIS SPENDS MODEL QUOTA (Linggen's model) AND CLAUDE CODE USAGE. A live run
asks for --yes. `--dry-run` spends nothing: it applies each task's canned
good and bad change (and an untouched copy) and shows every hidden check
passing and failing.

  /usr/bin/python3 evals/code/run.py --dry-run
  /usr/bin/python3 evals/code/run.py --yes --model <id> [--runs 3] [--arms code,plain]

Python 3 standard library only.
"""

import argparse
import base64
import datetime
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check  # noqa: E402

HERE = Path(__file__).resolve().parent
LIVE_ARMS = ["code", "plain", "claude"]
DRY_ARMS = ["dry-none", "dry-good", "dry-bad"]
DRY_EXPECT = {"dry-none": False, "dry-good": True, "dry-bad": False}
DRY_DIR = {"dry-good": "good", "dry-bad": "bad"}

DEFAULT_ENGINE = "http://127.0.0.1:9527"
# Build tools the Claude arm may run without asking (as allowed Bash). The
# Linggen arms both get admin on their scratch copy (`permission_mode: auto`),
# so the two differ by the work section alone, not by what they may run;
# what leaves the machine still asks, and is answered deny.
DEFAULT_TOOLS = "python3,node,cargo,rustc,sh"
ASK_TIMEOUT_S = 15
MCP_CALL_TIMEOUT_S = 150  # agent_run itself waits up to ~45 s before it hands back


# ======================================================================
# Linggen arm: /mcp (JSON-RPC over POST) and the wait socket
# ======================================================================

class McpError(Exception):
    pass


class Mcp:
    """The engine's /mcp endpoint: stateless JSON-RPC, one POST per call."""

    def __init__(self, engine):
        self.engine = engine.rstrip("/")
        self.n = 0

    def rpc(self, method, params=None, timeout=MCP_CALL_TIMEOUT_S):
        self.n += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method,
                           "params": params or {}}).encode()
        req = urllib.request.Request(
            self.engine + "/mcp", data=body, method="POST",
            headers={"Content-Type": "application/json",
                     "Accept": "application/json, text/event-stream"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                raw = r.read().decode("utf-8", errors="replace")
        except (urllib.error.URLError, OSError) as e:
            raise McpError("cannot reach %s/mcp: %s" % (self.engine, e))
        msg = json.loads(raw)
        if "error" in msg:
            raise McpError("%s: %s" % (method, msg["error"].get("message")))
        return msg.get("result", {})

    def call(self, tool, args, timeout=MCP_CALL_TIMEOUT_S):
        """-> (text, is_error)."""
        result = self.rpc("tools/call", {"name": tool, "arguments": args}, timeout)
        text = "".join(c.get("text", "") for c in result.get("content", []))
        return text, bool(result.get("isError"))

    def call_json(self, tool, args):
        text, is_err = self.call(tool, args)
        if is_err:
            raise McpError("%s: %s" % (tool, text))
        try:
            return json.loads(text)
        except ValueError:
            raise McpError("%s returned non-JSON: %s" % (tool, text[:200]))


class WebSocket:
    """Just enough RFC 6455 client for the engine's /api/agent/wait stream."""

    def __init__(self, url):
        u = urllib.parse.urlparse(url)
        self.sock = socket.create_connection((u.hostname, u.port or 80), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        path = u.path + ("?" + u.query if u.query else "")
        self.sock.sendall((
            "GET %s HTTP/1.1\r\nHost: %s:%s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
            "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n"
            % (path, u.hostname, u.port or 80, key)).encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise McpError("wait socket closed during handshake")
            head += chunk
        head, _, self.buf = head.partition(b"\r\n\r\n")
        status = head.split(b"\r\n", 1)[0]
        if b" 101 " not in status + b" ":
            raise McpError("wait socket refused: %s" % status.decode(errors="replace"))

    def _need(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _send(self, opcode, payload=b""):
        mask = os.urandom(4)
        head = bytes([0x80 | opcode])
        n = len(payload)
        head += bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + n.to_bytes(2, "big")
        self.sock.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def recv(self, timeout):
        """Next text message; None when the server closed; socket.timeout on silence."""
        self.sock.settimeout(timeout)
        message = b""
        try:
            while True:
                b0, b1 = self._need(2)
                opcode, n = b0 & 0x0F, b1 & 0x7F
                if n == 126:
                    n = int.from_bytes(self._need(2), "big")
                elif n == 127:
                    n = int.from_bytes(self._need(8), "big")
                payload = self._need(n)  # server frames are unmasked
                if opcode == 0x9:
                    self._send(0xA, payload)
                elif opcode == 0x8:
                    return None
                elif opcode in (0x1, 0x0):
                    message += payload
                    if b0 & 0x80:
                        return message.decode("utf-8", errors="replace")
        except ConnectionError:
            return None

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def ws_url(engine, session):
    u = urllib.parse.urlparse(engine)
    return "ws://%s/api/agent/wait?session_id=%s" % (u.netloc, urllib.parse.quote(session))


SESSION_RE = re.compile(r"Linggen session ([^\s.]+)")
GRANTS_LINE = re.compile(r"\n*Preset grants:.*\Z", re.S)


def run_linggen(args, work, ask, work_dir):
    """One delegated run. Returns the outcome dict (see `outcome`)."""
    mcp = Mcp(args.engine)
    t0 = time.time()
    deadline = t0 + args.timeout
    call_args = {
        "prompt": ask,
        "write_dir": str(work_dir),
        "permission_mode": "auto",
        "work": work,
        "model_fallback": False,
    }
    if args.model:
        call_args["model"] = args.model
    out = outcome()
    try:
        text, is_err = mcp.call("agent_run", call_args)
        if is_err:
            out["error"] = "agent_run: " + text[:300]
            return finish(out, t0)
        m = SESSION_RE.search(text) if text.startswith("Still running") else None
        if not m:
            # Finished inside the first call: the text is the reply (denials
            # are appended to it by the engine).
            out.update(reply=GRANTS_LINE.sub("", text).strip(), status="completed", asks=0)
            return finish(out, t0)
        session = m.group(1)
        out["session"] = session
        watch(mcp, args.engine, session, deadline, out)
    except McpError as e:
        out["error"] = str(e)
    except (OSError, ValueError) as e:
        out["error"] = "engine call failed: %s" % e
    return finish(out, t0)


def watch(mcp, engine, session, deadline, out):
    """Follow a running session to its end: answer every ask 'deny', record it.

    The wait socket is the main path; every time it closes, expires or goes
    quiet for 60 s the session's status is read once, so a missed `finished`
    cannot leave the runner waiting.
    """
    asks = {}
    final = None

    def deny(request):
        rid = request.get("id")
        if not rid or rid in asks:
            return
        asks[rid] = {k: request.get(k) for k in ("tool", "summary", "level", "path", "reason")}
        try:
            mcp.call("agent_permit", {"session_id": session, "request_id": rid, "decision": "deny"})
        except McpError:
            pass  # already answered or expired

    while final is None and time.time() < deadline:
        ws = None
        try:
            ws = WebSocket(ws_url(engine, session))
            while final is None and time.time() < deadline:
                raw = ws.recv(min(60, max(1, deadline - time.time())))
                if raw is None:
                    break
                frame = json.loads(raw)
                if frame.get("type") == "permission_request":
                    deny(frame.get("request") or {})
                elif frame.get("type") == "finished":
                    final = frame
        except socket.timeout:
            pass
        except (McpError, OSError, ValueError):
            time.sleep(2)
        finally:
            if ws:
                ws.close()
        if final is None:
            st = mcp.call_json("agent_status", {"session_id": session})
            for request in st.get("pending") or []:
                deny(request)
            if not st.get("running") and st.get("status") and not st.get("pending"):
                final = st
    if final is None:
        out["error"] = "timed out"
        try:
            mcp.call("agent_stop", {"session_id": session})
        except McpError:
            pass
    st = mcp.call_json("agent_status", {"session_id": session})
    for request in st.get("pending") or []:
        deny(request)
    reply = (final or {}).get("reply") or st.get("reply") or ""
    out.update(reply=reply, status=(final or st).get("status") or st.get("status"),
               model=(final or st).get("model") or st.get("model"), asks=len(asks),
               ask_log=list(asks.values()), denied=st.get("denied") or [])
    if out["status"] == "failed":
        out["error"] = (final or st).get("error") or st.get("error") or "failed"
    try:
        mcp.call("agent_session_close", {"session_id": session})
    except McpError:
        pass


def preflight_linggen(args):
    """Fail early and plainly when the engine cannot take these runs."""
    mcp = Mcp(args.engine)
    mcp.rpc("initialize", {}, timeout=ASK_TIMEOUT_S)
    tools = {t["name"]: t for t in mcp.rpc("tools/list", {}, timeout=ASK_TIMEOUT_S).get("tools", [])}
    run_tool = tools.get("agent_run")
    if not run_tool:
        raise McpError("this engine's /mcp has no agent_run")
    if "work" not in run_tool.get("inputSchema", {}).get("properties", {}):
        raise McpError("this engine's agent_run has no `work` option (older than 3.0)")
    if args.model:
        text, _ = mcp.call("agent_models", {}, timeout=ASK_TIMEOUT_S)
        if args.model not in text:
            print("warning: model %r not seen in agent_models:\n%s" % (args.model, text), file=sys.stderr)


# ======================================================================
# Claude Code arm
# ======================================================================

def claude_command(args):
    allowed = ",".join("Bash(%s *)" % t for t in args.tools.split(",") if t)
    cmd = [args.claude_bin, "-p", "--output-format", "stream-json", "--verbose",
           "--permission-mode", "acceptEdits", "--allowedTools", allowed,
           "--no-session-persistence", "--strict-mcp-config", "--disable-slash-commands",
           "--setting-sources", "project,local"]
    if args.claude_model:
        cmd += ["--model", args.claude_model]
    if args.claude_args:
        cmd += args.claude_args.split()
    return cmd


def run_claude(args, ask, work_dir):
    t0 = time.time()
    out = outcome()
    try:
        # The prompt goes in on stdin so the variadic --allowedTools cannot swallow it.
        proc = subprocess.run(claude_command(args), input=ask.encode(), cwd=str(work_dir),
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=args.timeout)
    except subprocess.TimeoutExpired as e:
        out.update(error="timed out", transcript=(e.stdout or b"").decode("utf-8", errors="replace"))
        return finish(out, t0)
    except OSError as e:
        out["error"] = "cannot run %s: %s" % (args.claude_bin, e)
        return finish(out, t0)
    text = proc.stdout.decode("utf-8", errors="replace")
    out["transcript"] = text
    tool_calls, result = 0, None
    for line in text.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "assistant":
            for block in (event.get("message") or {}).get("content") or []:
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    tool_calls += 1
        elif event.get("type") == "result":
            result = event
    out["tool_calls"] = tool_calls
    if result is None:
        out["error"] = "no result event (exit %s): %s" % (proc.returncode, proc.stderr.decode(errors="replace")[-300:])
        return finish(out, t0)
    denials = result.get("permission_denials") or []
    out.update(reply=result.get("result") or "", status="failed" if result.get("is_error") else "completed",
               asks=len(denials), ask_log=denials, model=args.claude_model)
    if result.get("is_error"):
        out["error"] = str(result.get("subtype") or "error")
    return finish(out, t0)


# ======================================================================
# Dry-run arms (no engine, no model)
# ======================================================================

def run_dry(arm, task, work_dir):
    t0 = time.time()
    out = outcome()
    if arm == "dry-none":
        out.update(reply="", status="completed")
        return finish(out, t0)
    canned = task["dir"] / "canned" / DRY_DIR[arm]
    for path in sorted(canned.rglob("*")):
        rel = path.relative_to(canned)
        if path.is_file() and not rel.name.startswith("_"):
            target = work_dir / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
    delete = canned / "_delete.txt"
    if delete.exists():
        for rel in delete.read_text().split():
            (work_dir / rel).unlink()
    reply = canned / "_reply.txt"
    out.update(reply=reply.read_text() if reply.exists() else "", status="completed")
    return finish(out, t0)


# ======================================================================
# One cell, and the whole matrix
# ======================================================================

def outcome():
    return {"reply": None, "status": None, "error": None, "model": None, "session": None,
            "tool_calls": None, "asks": None, "ask_log": [], "denied": [], "transcript": None,
            "wall_s": 0.0}


def finish(out, t0):
    out["wall_s"] = round(time.time() - t0, 1)
    return out


def run_cell(args, task, arm, k, scratch, results_dir):
    work = Path(os.path.realpath(tempfile.mkdtemp(prefix="%s-%s-" % (task["name"], arm), dir=scratch)))
    shutil.copytree(task["dir"] / "project", work, dirs_exist_ok=True)
    original = check.read_tree(work)
    ask = task["ask"]
    if arm in ("code", "plain"):
        out = run_linggen(args, "code" if arm == "code" else "none", ask, work)
    elif arm == "claude":
        out = run_claude(args, ask, work)
    else:
        out = run_dry(arm, task, work)
    scored = check.score(task, work, original, out["reply"] or "")
    if out["error"]:
        scored["passed"] = False
        scored["failed"] = ["error"] + [f for f in scored["failed"] if f != "error"]
        scored["detail"]["error"] = out["error"]
    record = {
        "task": task["name"], "arm": arm, "run": k, "passed": scored["passed"],
        "failed": scored["failed"], "detail": scored["detail"],
        "tests_changed": scored["tests_changed"], "added": scored["added"], "removed": scored["removed"],
        "code_added": scored["code_added"], "code_removed": scored["code_removed"],
        "files": scored["files"], "wall_s": out["wall_s"], "tool_calls": out["tool_calls"],
        "asks": out["asks"], "status": out["status"], "error": out["error"], "model": out["model"],
        "session": out["session"],
    }
    save_cell(results_dir, record, out, scored)
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    else:
        record["kept"] = str(work)
    return record


def save_cell(results_dir, record, out, scored):
    d = results_dir / record["task"] / record["arm"] / ("run%d" % record["run"])
    d.mkdir(parents=True, exist_ok=True)
    (d / "reply.txt").write_text(out["reply"] or "")
    (d / "diff.patch").write_text(scored["patch"])
    (d / "check.txt").write_text(scored["check_output"])
    extra = {"ask_log": out["ask_log"], "denied": out["denied"], **record}
    (d / "record.json").write_text(json.dumps(extra, indent=2))
    if out["transcript"]:
        (d / "transcript.jsonl").write_text(out["transcript"])


# ======================================================================
# The table
# ======================================================================

def mean(values):
    values = [v for v in values if v is not None]
    return sum(values) / len(values) if values else None


def fmt(v, digits=1):
    return "n/a" if v is None else ("%.*f" % (digits, v))


def cell_text(records, task):
    if not records:
        return "-"
    n, p = len(records), sum(r["passed"] for r in records)
    text = ("pass" if p else "FAIL") if n == 1 else "%d/%d" % (p, n)
    return text + ("*" if task.get("judged_by_person") else "")


def render_markdown(meta, tasks, arms, records):
    by = {}
    for r in records:
        by.setdefault((r["task"], r["arm"]), []).append(r)
    lines = ["# Code eval - %s" % meta["when"], ""]
    lines.append("Engine: `%s`  Model: `%s`  Claude model: `%s`  Runs per cell: %d%s" % (
        meta["engine"], meta["model"] or "(engine default)", meta["claude_model"] or "(Claude Code default)",
        meta["runs"], "  **DRY RUN**" if meta["dry_run"] else ""))
    lines += ["", "A model is not the same twice: compare means over several runs, not one pass.",
              "`*` = the pass rule reads the final reply with a plain text rule; read the saved reply too.", ""]
    lines.append("| task | " + " | ".join(arms) + " |")
    lines.append("|:--|" + "|".join([":-:"] * len(arms)) + "|")
    for t in tasks:
        lines.append("| %s | %s |" % (t["name"], " | ".join(cell_text(by.get((t["name"], a)), t) for a in arms)))
    totals = []
    for a in arms:
        rs = [r for r in records if r["arm"] == a]
        totals.append("%d/%d" % (sum(r["passed"] for r in rs), len(rs)))
    lines.append("| **total** | %s |" % " | ".join("**%s**" % t for t in totals))

    lines += ["", "## Measures per arm", "",
              "| arm | runs | passed | test file changed | mean lines added | mean lines removed | mean wall s | mean tool calls | permission asks | errors |",
              "|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|"]
    for a in arms:
        rs = [r for r in records if r["arm"] == a]
        asks = [r["asks"] for r in rs if r["asks"] is not None]
        lines.append("| %s | %d | %d | %d | %s | %s | %s | %s | %s | %d |" % (
            a, len(rs), sum(r["passed"] for r in rs), sum(r["tests_changed"] for r in rs),
            fmt(mean([r["added"] for r in rs])), fmt(mean([r["removed"] for r in rs])),
            fmt(mean([r["wall_s"] for r in rs])), fmt(mean([r["tool_calls"] for r in rs])),
            str(sum(asks)) if asks else "n/a", sum(1 for r in rs if r["error"])))
    lines += ["", "`n/a` = the arm does not report it (Linggen's `agent_status` carries no tool-call count).", "",
              "## Every run", "",
              "| task | arm | run | result | why it failed | +/- lines | test changed | wall s | asks |",
              "|:--|:--|--:|:--|:--|:--|:--|--:|--:|"]
    for r in records:
        why = "; ".join("%s: %s" % (k, v) for k, v in r["detail"].items()) if not r["passed"] else ""
        lines.append("| %s | %s | %d | %s | %s | +%d/-%d | %s | %s | %s |" % (
            r["task"], r["arm"], r["run"], "pass" if r["passed"] else "FAIL", why.replace("|", "/")[:160],
            r["added"], r["removed"], "yes" if r["tests_changed"] else "no", fmt(r["wall_s"]),
            "n/a" if r["asks"] is None else r["asks"]))
    lines += ["", "## What each task is there to catch", ""]
    for t in tasks:
        lines.append("- **%s** (%s): %s" % (t["name"], t["lang"], t["catches"]))
    return "\n".join(lines) + "\n"


def dry_verdicts(tasks, records):
    """(rows, all_ok): does every canned change behave as its task says?"""
    rows, ok_all = [], True
    for t in tasks:
        for arm in DRY_ARMS:
            r = next(x for x in records if x["task"] == t["name"] and x["arm"] == arm)
            if arm == "dry-bad":
                want = sorted(t["bad_fails"])
                ok = (not r["passed"]) and sorted(r["failed"]) == want
                expect = "fail by " + ",".join(want)
            else:
                ok = r["passed"] == DRY_EXPECT[arm]
                expect = "pass" if DRY_EXPECT[arm] else "fail"
            ok_all &= ok
            rows.append((t["name"], arm, expect, "pass" if r["passed"] else "fail by " + ",".join(r["failed"]), ok))
    return rows, ok_all


def render_dry(rows):
    out = ["", "| task | change | expected | got | |", "|:--|:--|:--|:--|:-:|"]
    for task, arm, expect, got, ok in rows:
        out.append("| %s | %s | %s | %s | %s |" % (task, arm, expect, got, "ok" if ok else "WRONG"))
    return "\n".join(out) + "\n"


# ======================================================================
# main
# ======================================================================

def parse_args(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dry-run", action="store_true", help="apply canned changes; no engine, no model, no claude")
    ap.add_argument("--yes", action="store_true", help="a live run spends model quota and Claude Code usage: confirm it")
    ap.add_argument("--engine", default=DEFAULT_ENGINE, help="Linggen engine address (default %(default)s)")
    ap.add_argument("--model", help="Linggen model id for the code and plain arms (agent_models lists them)")
    ap.add_argument("--claude-model", help="model for the claude arm (default: Claude Code's own)")
    ap.add_argument("--claude-bin", default="claude", help="the Claude Code executable (default %(default)s)")
    ap.add_argument("--claude-args", default="", help="extra flags for claude, split on spaces")
    ap.add_argument("--tools", default=DEFAULT_TOOLS, help="build tools the Claude arm may run unasked (default %(default)s)")
    ap.add_argument("--arms", default=",".join(LIVE_ARMS), help="comma list from: " + ",".join(LIVE_ARMS))
    ap.add_argument("--tasks", help="comma list of task names (default: all)")
    ap.add_argument("--runs", type=int, default=1, help="repeats per task and arm (default 1)")
    ap.add_argument("--timeout", type=int, default=900, help="seconds one run may take (default %(default)s)")
    ap.add_argument("--scratch", help="parent folder for the working copies (default: the system temp folder)")
    ap.add_argument("--out", help="results folder (default: evals/code/results/<timestamp>)")
    ap.add_argument("--keep", action="store_true", help="keep the working copies")
    return ap.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    names = args.tasks.split(",") if args.tasks else check.all_task_names()
    tasks = [check.load_task(n) for n in names]
    arms = DRY_ARMS if args.dry_run else [a for a in args.arms.split(",") if a]
    bad = [a for a in arms if a not in DRY_ARMS + LIVE_ARMS]
    if bad or args.runs < 1:
        sys.exit("unknown arm %s or --runs < 1" % bad)
    cells = len(tasks) * len(arms) * args.runs
    if not args.dry_run:
        print("This is a LIVE run: %d runs (%d tasks x %d arms x %d), each spending model quota"
              " (Linggen arms) or Claude Code usage (claude arm)." % (cells, len(tasks), len(arms), args.runs))
        if not args.yes:
            sys.exit("Not started. Add --yes to go ahead, or --dry-run to see the harness work for free.")
        if {"code", "plain"} & set(arms):
            try:
                preflight_linggen(args)
            except McpError as e:
                sys.exit("engine not ready: %s" % e)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    results_dir = Path(args.out) if args.out else HERE / "results" / (stamp + ("-dry" if args.dry_run else ""))
    results_dir.mkdir(parents=True, exist_ok=True)
    scratch_root = os.path.realpath(tempfile.mkdtemp(prefix="linggen-codeeval-", dir=args.scratch))
    records = []
    try:
        for t in tasks:
            for a in arms:
                for k in range(1, args.runs + 1):
                    r = run_cell(args, t, a, k, scratch_root, results_dir)
                    records.append(r)
                    print("%-26s %-8s run %d  %s%s" % (t["name"], a, k, "pass" if r["passed"] else "FAIL",
                                                      "" if r["passed"] else "  (" + ",".join(r["failed"]) + ")"), flush=True)
    finally:
        if not args.keep:
            shutil.rmtree(scratch_root, ignore_errors=True)
    meta = {"when": stamp, "engine": "(none)" if args.dry_run else args.engine, "model": args.model,
            "claude_model": args.claude_model, "runs": args.runs, "dry_run": args.dry_run}
    md = render_markdown(meta, tasks, arms, records)
    rows = None
    if args.dry_run:
        rows, ok_all = dry_verdicts(tasks, records)
        md += "\n## Dry-run expectations\n" + render_dry(rows)
    (results_dir / "table.md").write_text(md)
    (results_dir / "results.json").write_text(json.dumps({"meta": meta, "records": records}, indent=2))
    print("\n" + md)
    print("written to", results_dir)
    if args.dry_run:
        good = sum(1 for r in rows if r[4])
        print("dry-run: %d/%d checks behave as expected (%d tasks: untouched fails, good passes, bad fails for its reason)"
              % (good, len(rows), len(tasks)))
        sys.exit(0 if ok_all else 1)


if __name__ == "__main__":
    main()
