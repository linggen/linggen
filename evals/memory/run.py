#!/usr/bin/env python3
"""Write-side memory eval runner.

Drives each scenario through a REAL engine instance over HTTP (production
prompt assembly, per-turn recall, capture protocol, real dream mission)
against a THROWAWAY store. Emits one end-state JSON per scenario for the
checker (check.py). The user's real store is never touched.

Usage:
  python3 evals/memory/run.py [--filter NAME] [--keep] [--out DIR]

Requires: the workspace release binary (target/release/ling), ling-mem on
PATH, PyYAML. Ports 9871 (engine) / 9872 (ling-mem) must be free.
"""

import argparse
import json
import os
import random
import shutil
import signal
import string
import subprocess
import sys
import tempfile
import time
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
ENGINE_BIN = ROOT / "target/release/ling"
ENGINE_PORT = 9871
LINGMEM_PORT = 9872
ENGINE = f"http://127.0.0.1:{ENGINE_PORT}"
LINGMEM = f"http://127.0.0.1:{LINGMEM_PORT}"

TURN_TIMEOUT_S = 300
DREAM_TIMEOUT_S = 600


def http(method, url, body=None, timeout=30):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=timeout) as r:
        raw = r.read()
    if not raw:
        return None
    parsed = json.loads(raw)
    if isinstance(parsed, dict) and "data" in parsed and ("ok" in parsed or len(parsed) == 1):
        return parsed["data"]
    return parsed


def wait_health(url, secs=30):
    for _ in range(secs * 2):
        try:
            http("GET", url + "/api/health", timeout=2)
            return True
        except Exception:
            time.sleep(0.5)
    return False


def short_id():
    return "".join(random.choices(string.ascii_letters + string.digits, k=10))


class Stack:
    """One throwaway ling-mem + engine pair, rebuilt per scenario."""

    def __init__(self, keep=False):
        self.keep = keep
        self.dir = Path(tempfile.mkdtemp(prefix="linggen-memeval-"))
        self.store_dir = self.dir / "store"
        self.ws_dir = self.dir / "workspace"
        # Isolated engine home: sessions, config, agents, mission run
        # records all live here — nothing touches the real ~/.linggen.
        self.home = self.dir / "home"
        self.store_dir.mkdir()
        self.ws_dir.mkdir()
        (self.home / "config").mkdir(parents=True)
        self.procs = []

    def import_fixture(self, rows):
        """Direct-store import (daemon not yet running) — the only path
        that honors backdated created_at, which the TTL/day logic needs."""
        env = {**os.environ, "LINGGEN_DATA_DIR": str(self.store_dir)}
        for episodic in (False, True):
            batch = [r for r in rows
                     if (r.get("tier", "semantic") == "episodic") == episodic]
            if not batch:
                continue
            ts_rows = []
            for r in batch:
                ts = (datetime.now(timezone.utc)
                      - timedelta(days=r.get("age_days", 0))).isoformat()
                ts_rows.append(json.dumps({
                    "id": short_id(),
                    "content": r["content"],
                    "contexts": r.get("contexts", []),
                    "tags": [],
                    "type": r.get("type", "fact"),
                    "tier": r.get("tier", "semantic"),
                    "from": r.get("from", "derived"),
                    "created_at": ts,
                    "occurred_at": ts,
                    "host": "memeval-fixture",
                }))
            nd = self.dir / ("fixture-epi.ndjson" if episodic else "fixture-sem.ndjson")
            nd.write_text("\n".join(ts_rows) + "\n")
            cmd = ["ling-mem"] + (["--episodic"] if episodic else []) + \
                ["import", str(nd)]
            subprocess.run(cmd, env=env, check=True,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def start(self):
        env = {**os.environ, "LINGGEN_DATA_DIR": str(self.store_dir)}
        self.procs.append(
            subprocess.Popen(
                ["ling-mem", "serve", "--port", str(LINGMEM_PORT)],
                env=env,
                stdout=(self.dir / "ling-mem.log").open("w"),
                stderr=subprocess.STDOUT,
            )
        )
        if not wait_health(LINGMEM):
            raise RuntimeError("throwaway ling-mem did not become healthy")

        (self.home / "config/linggen.runtime.toml").write_text(self._eval_config())
        # Built-in missions (dream) are hand-installed, not binary-embedded
        # like agents — copy them in so decay scenarios can trigger dream.
        real_missions = Path.home() / ".linggen/missions"
        if real_missions.exists():
            shutil.copytree(real_missions, self.home / "missions",
                            dirs_exist_ok=True)
        # Model auth lives under the engine home (e.g. the ChatGPT OAuth
        # tokens) — copy it in or every model call fails AUTH_REQUIRED.
        for auth in Path.home().glob(".linggen/*auth*.json"):
            shutil.copy(auth, self.home / auth.name)
        env = {**os.environ, "LINGGEN_HOME": str(self.home)}
        env.pop("LINGGEN_CONFIG", None)
        self.procs.append(
            subprocess.Popen(
                [str(ENGINE_BIN), "--web", "--port", str(ENGINE_PORT),
                 "--root", str(self.ws_dir)],
                env=env,
                stdout=(self.dir / "engine.log").open("w"),
                stderr=subprocess.STDOUT,
            )
        )
        if not wait_health(ENGINE):
            raise RuntimeError("eval engine did not become healthy")

    def _eval_config(self):
        # Start from the user's real config (model providers / auth
        # references live there), then point memory at the throwaway
        # daemon. Text-level patch: drop any existing ling_mem_url line
        # and re-add it under [agent].
        base = ""
        for cand in [
            Path.home() / ".linggen/config/linggen.runtime.toml",
            Path.home() / ".linggen/config/linggen.toml",
        ]:
            if cand.exists():
                base = cand.read_text()
                break
        if not base:
            raise RuntimeError("no real linggen config found to base the eval config on")
        lines = [l for l in base.splitlines() if "ling_mem_url" not in l]
        if not any(l.strip() == "[agent]" for l in lines):
            lines.append("[agent]")
        out = []
        for l in lines:
            out.append(l)
            if l.strip() == "[agent]":
                out.append(f'ling_mem_url = "{LINGMEM}"')
        return "\n".join(out) + "\n"

    def stop(self):
        for p in self.procs:
            try:
                p.send_signal(signal.SIGTERM)
            except Exception:
                pass
        for p in self.procs:
            try:
                p.wait(timeout=10)
            except Exception:
                p.kill()
        if not self.keep:
            shutil.rmtree(self.dir, ignore_errors=True)


def session_file(home, session_id):
    return home / "sessions" / session_id / "messages.jsonl"


def count_assistant(path):
    if not path.exists():
        return 0
    n = 0
    for line in path.read_text().splitlines():
        try:
            m = json.loads(line)
            if m.get("to_id") != "user" or m.get("from_id") in (None, "user", "system"):
                continue
            # Tool calls/results are persisted as JSON-typed content
            # addressed to the user — only plain prose is the final reply.
            c = m.get("content", "")
            try:
                if isinstance(json.loads(c), dict):
                    continue
            except Exception:
                pass
            n += 1
        except Exception:
            pass
    return n


def run_turn(stack, session_id, message, ask_cfg, events):
    """Send one user turn; wait for the assistant reply, answering an
    AskUser widget along the way if one fires."""
    body = {"project_root": str(stack.ws_dir), "agent_id": "ling",
            "message": message}
    if session_id:
        body["session_id"] = session_id
    # Baseline BEFORE the POST — a fast reply landing between POST and
    # the first count would otherwise be absorbed into `before`.
    before = count_assistant(session_file(stack.home, session_id)) if session_id else 0
    resp = http("POST", ENGINE + "/api/chat", body)
    session_id = resp.get("session_id", session_id)
    sfile = session_file(stack.home, session_id)

    deadline = time.time() + TURN_TIMEOUT_S
    while time.time() < deadline:
        pending = []
        try:
            pending = http("GET", ENGINE + "/api/pending-ask-user") or []
        except Exception:
            pass
        mine = [p for p in pending if p.get("session_id") == session_id]
        if mine:
            q = mine[0]
            events.append({"ask_user": q["questions"]})
            answer = (ask_cfg or {}).get("answer", "")
            http("POST", ENGINE + "/api/ask-user-response", {
                "question_id": q["question_id"],
                "answers": [{
                    "question_index": 0,
                    "selected": [],
                    "custom_text": answer,
                }],
            })
        if count_assistant(sfile) > before:
            return session_id
        time.sleep(2)
    raise RuntimeError(f"turn timed out: {message[:50]}")


def run_dream(events):
    resp = http("POST", ENGINE + "/api/missions/dream/trigger", {})
    sid = resp.get("session_id")
    deadline = time.time() + DREAM_TIMEOUT_S
    while time.time() < deadline:
        resp = http("GET", ENGINE + "/api/missions/dream/runs")
        runs = resp["runs"] if isinstance(resp, dict) else resp
        mine = [r for r in runs if r.get("session_id") == sid]
        if mine and mine[0]["status"] in ("completed", "failed", "stopped"):
            events.append({"dream": mine[0]["status"]})
            return
        time.sleep(5)
    raise RuntimeError("dream run timed out")


def dump_store():
    sem = http("POST", LINGMEM + "/api/memory/list", {"limit": 1000})
    epi = http("POST", LINGMEM + "/api/memory/list",
               {"limit": 1000, "episodic": True})
    for r in sem + epi:
        r.pop("vector", None)
    return {"semantic": sem, "episodic": epi}


def run_scenario(scn, out_dir, keep):
    stack = Stack(keep=keep)
    events = []
    error = None
    try:
        if scn.get("fixture"):
            stack.import_fixture(scn["fixture"])
        stack.start()
        session_id = None
        for turn in scn.get("turns", []):
            session_id = run_turn(stack, session_id, turn,
                                  scn.get("ask_user"), events)
        if scn.get("dream"):
            run_dream(events)
        state = dump_store()
    except Exception as e:  # noqa: BLE001 — the checker scores failures too
        error = str(e)
        state = {"semantic": [], "episodic": []}
    finally:
        stack.stop()

    result = {
        "name": scn["name"],
        "axis": scn["axis"],
        "expect": scn.get("expect", {}),
        "ask_user_cfg": scn.get("ask_user"),
        "events": events,
        "error": error,
        "store": state,
    }
    out = out_dir / f"{scn['name']}.json"
    out.write_text(json.dumps(result, indent=2, ensure_ascii=False))
    status = "ERROR" if error else "ok"
    print(f"  {scn['name']:<32} {status}")
    return result


def load_scenarios(filter_):
    scenarios = []
    for f in sorted((ROOT / "evals/memory/scenarios").glob("*.yaml")):
        scenarios.extend(yaml.safe_load(f.read_text()) or [])
    if filter_:
        scenarios = [s for s in scenarios if filter_ in s["name"]]
    return scenarios


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter")
    ap.add_argument("--keep", action="store_true",
                    help="keep throwaway dirs for debugging")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    out_dir = Path(args.out) if args.out else \
        ROOT / "evals/memory/results" / datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir.mkdir(parents=True, exist_ok=True)

    scenarios = load_scenarios(args.filter)
    print(f"running {len(scenarios)} scenario(s) → {out_dir}")
    for scn in scenarios:
        run_scenario(scn, out_dir, args.keep)
    print(f"done. score with: python3 evals/memory/check.py {out_dir}")


if __name__ == "__main__":
    main()
