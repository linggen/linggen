#!/usr/bin/env python3
"""Write-side memory eval checker — scores runner output.

Mechanical checks run first (contains-traps, AskUser fired, tier
placement); judged predicates go through a model on a THROWAWAY engine
stack (its capture writes land in its own throwaway store, never the
real one). Every judged verdict cites row ids, so failures are
debuggable and spot-checkable.

Usage:
  python3 evals/memory/check.py <results-dir> [--filter NAME]
"""

import argparse
import json
import sys
import time
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from run import Stack, http, ENGINE, run_turn  # noqa: E402

JUDGE_PREAMBLE = (
    "You are a strict binary evaluator for a memory-store test. "
    "Below are memory rows in the form `id | tier | content`. "
    "Do not use any other knowledge; judge only from the rows shown. "
)


class Judge:
    """One throwaway engine stack shared by all judged checks."""

    def __init__(self):
        self.stack = None

    def ensure(self):
        if self.stack is None:
            self.stack = Stack(keep=False)
            self.stack.start()

    def close(self):
        if self.stack is not None:
            self.stack.stop()

    def ask(self, prompt):
        self.ensure()
        # Fresh session per question — no context bleed between checks.
        events = []
        sid = run_turn(self.stack, None, prompt, None, events)
        sfile = self.stack.home / "sessions" / sid / "messages.jsonl"
        last = ""
        for line in sfile.read_text().splitlines():
            try:
                m = json.loads(line)
            except Exception:
                continue
            if m.get("to_id") != "user" or m.get("from_id") in (None, "user", "system"):
                continue
            c = m.get("content", "")
            try:
                if isinstance(json.loads(c), dict):
                    continue
            except Exception:
                pass
            last = c
        return last.strip()


def rows_block(rows):
    return "\n".join(
        f"{r['id']} | {r.get('tier','?')} | {r['content']}" for r in rows
    )


def pick_rows(store, tier=None):
    rows = store["semantic"] + store["episodic"]
    if tier == "episodic":
        rows = store["episodic"]
    elif tier in ("core", "semantic"):
        rows = [r for r in store["semantic"] if r.get("tier") == tier]
    return rows


def judge_exists(judge, rows, predicate):
    """Returns (exists: bool, cited_id)."""
    if not rows:
        return False, None
    prompt = (
        JUDGE_PREAMBLE
        + f"Question: does any row genuinely assert the following?\n"
        + f"PREDICATE: {predicate}\n\nROWS:\n{rows_block(rows)}\n\n"
        + "Reply with EXACTLY one line: `YES <row-id>` or `NO`. Nothing else."
    )
    reply = judge.ask(prompt)
    first = reply.splitlines()[-1].strip() if reply else "NO"
    if first.upper().startswith("YES"):
        parts = first.split()
        return True, parts[1] if len(parts) > 1 else None
    return False, None


def judge_subject_count(judge, rows, subject):
    """Returns list of row ids asserting a current value for the subject."""
    if not rows:
        return []
    prompt = (
        JUDGE_PREAMBLE
        + f"Question: which rows each independently assert a CURRENT value for this subject?\n"
        + f"SUBJECT: {subject}\n\nROWS:\n{rows_block(rows)}\n\n"
        + "Reply with EXACTLY one line: the matching row ids separated by "
        + "spaces, or `NONE`. Nothing else."
    )
    reply = judge.ask(prompt)
    last = reply.splitlines()[-1].strip() if reply else "NONE"
    if last.upper() == "NONE":
        return []
    known = {r["id"] for r in rows}
    return [t for t in last.split() if t in known]


def check_scenario(result, judge):
    checks = []  # (kind, description, passed, detail)

    def add(kind, desc, passed, detail=""):
        checks.append({"kind": kind, "desc": desc,
                       "passed": bool(passed), "detail": detail})

    if result.get("error"):
        add("runner", "scenario ran without runner errors", False,
            result["error"])
        return checks

    store = result["store"]
    expect = result.get("expect", {})
    all_rows = store["semantic"] + store["episodic"]

    # AskUser expectation (mechanical, from run events)
    ask_cfg = result.get("ask_user_cfg") or {}
    fired = any("ask_user" in e for e in result.get("events", []))
    want = bool(ask_cfg.get("expect", False))
    add("ask_user", f"AskUser fired={want}", fired == want,
        f"fired={fired}")

    # must_not contains — mechanical substring
    for item in expect.get("must_not", []):
        if "contains" in item:
            token = item["contains"]
            hits = [r["id"] for r in all_rows if token in r["content"]]
            add("trap", f"no row contains {token!r}", not hits,
                f"hits={hits}")

    # must predicates — judged
    for item in expect.get("must", []):
        rows = pick_rows(store, item.get("tier"))
        exists, cited = judge_exists(judge, rows, item["predicate"])
        tier_note = f" [{item['tier']}]" if item.get("tier") else ""
        add("must", item["predicate"][:80] + tier_note, exists,
            f"cited={cited}")

    # must_not predicates — judged
    for item in expect.get("must_not", []):
        if "predicate" in item:
            rows = pick_rows(store, item.get("tier"))
            exists, cited = judge_exists(judge, rows, item["predicate"])
            tier_note = f" [{item['tier']}]" if item.get("tier") else ""
            add("must_not", item["predicate"][:80] + tier_note, not exists,
                f"cited={cited}")

    # subject_max_one — judged count
    for subject in expect.get("subject_max_one", []):
        ids = judge_subject_count(judge, store["semantic"], subject)
        add("dedup", f"≤1 row asserts current {subject!r}", len(ids) <= 1,
            f"ids={ids}")

    return checks


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("results_dir")
    ap.add_argument("--filter")
    args = ap.parse_args()

    results_dir = Path(args.results_dir)
    files = sorted(results_dir.glob("*.json"))
    files = [f for f in files if f.name != "scorecard.json"
             and (not args.filter or args.filter in f.name)]

    judge = Judge()
    per_axis = defaultdict(lambda: [0, 0])  # axis -> [passed, total]
    must_tp = must_fn = trap_fp = 0
    report = []
    try:
        for f in files:
            result = json.loads(f.read_text())
            checks = check_scenario(result, judge)
            passed = all(c["passed"] for c in checks)
            per_axis[result["axis"]][0] += int(passed)
            per_axis[result["axis"]][1] += 1
            for c in checks:
                if c["kind"] == "must":
                    must_tp += int(c["passed"])
                    must_fn += int(not c["passed"])
                if c["kind"] in ("trap", "must_not"):
                    trap_fp += int(not c["passed"])
            report.append({"name": result["name"], "axis": result["axis"],
                           "passed": passed, "checks": checks})
            mark = "PASS" if passed else "FAIL"
            print(f"  {result['name']:<32} {mark}")
            for c in checks:
                if not c["passed"]:
                    print(f"      ✗ [{c['kind']}] {c['desc']}  ({c['detail']})")
    finally:
        judge.close()

    total = sum(t for _, t in per_axis.values())
    total_pass = sum(p for p, _ in per_axis.values())
    recall = must_tp / (must_tp + must_fn) if (must_tp + must_fn) else 1.0
    precision = must_tp / (must_tp + trap_fp) if (must_tp + trap_fp) else 1.0

    print("\n== scorecard ==")
    for axis, (p, t) in sorted(per_axis.items()):
        print(f"  {axis:<12} {p}/{t}")
    print(f"  overall      {total_pass}/{total}")
    print(f"  extraction precision {precision:.2f} · recall {recall:.2f}")

    scorecard = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "per_axis": {a: {"passed": p, "total": t}
                     for a, (p, t) in per_axis.items()},
        "overall": {"passed": total_pass, "total": total},
        "must_precision": precision,
        "must_recall": recall,
        "scenarios": report,
    }
    (results_dir / "scorecard.json").write_text(
        json.dumps(scorecard, indent=2, ensure_ascii=False))
    print(f"\nscorecard → {results_dir / 'scorecard.json'}")


if __name__ == "__main__":
    main()
