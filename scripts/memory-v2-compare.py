#!/usr/bin/env python3
"""Compare memory V1 with memory V2 on the last real Claude Code prompts.

Read-only, offline, no model calls. For each of the last N prompts typed in
Claude Code (from ~/.claude/projects/*/*.jsonl) it prints side by side:

  V1  what the plugin's per-turn recall put in front of the model then (the
      "From memory" lines already recorded in the log), and
  V2  what the forwarding would hand a session in that folder, by the
      engine's own code (the `outside_slice` example), as Claude Code
      itself would get it (it is never forwarded its own memory) and as a
      Codex session would.

Nothing is written but the markdown file in --out. Nothing outside the
logs, ~/.claude and ~/.codex is read, and those only for names, index files
(through the engine's reader) and the prompt text.

    python3 -I scripts/memory-v2-compare.py --out <dir> [--n 30] [--bin <path>]
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys

SKIP_PROJECT_DIRS = ("evals-scratch", "private-tmp", "cache-linggen", "-probe-", "hooktest")
SKIP_PROMPT_START = ("<", "[Request interrupted", "Caveat:")
KEY = re.compile(r"(sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|xox[abeprs]-[A-Za-z0-9-]{10,})")


def prompt_text(row):
    """The typed text of a user row, or None when it is not a typed prompt."""
    if row.get("type") != "user" or row.get("isSidechain") or "promptId" not in row:
        return None
    content = (row.get("message") or {}).get("content")
    if isinstance(content, list):
        content = "\n".join(b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text")
    if not isinstance(content, str):
        return None
    content = content.strip()
    if len(content) < 8 or content.startswith(SKIP_PROMPT_START):
        return None
    return content


def read_file(path):
    """(prompts, recalled): prompts [(ts, cwd, uuid, text)], recalled {uuid: [lines]}."""
    prompts, recalled = [], {}
    with open(path, errors="replace") as fh:
        for line in fh:
            if '"user"' not in line and "From memory (" not in line:
                continue
            try:
                row = json.loads(line)
            except ValueError:
                continue
            text = prompt_text(row)
            if text is not None:
                prompts.append((row.get("timestamp", ""), row.get("cwd", ""), row.get("uuid"), text))
                continue
            att = row.get("attachment")
            if row.get("type") == "attachment" and isinstance(att, dict):
                if att.get("type") == "hook_success" and att.get("hookEvent") == "UserPromptSubmit":
                    hits = [l for l in (att.get("stdout") or "").splitlines() if l.startswith("From memory (")]
                    if hits:
                        recalled.setdefault(row.get("parentUuid"), []).extend(hits)
    return prompts, recalled


def last_prompts(n):
    files = [
        f
        for f in glob.glob(os.path.expanduser("~/.claude/projects/*/*.jsonl"))
        if not any(s in f.split("/projects/")[1].split("/")[0] for s in SKIP_PROJECT_DIRS)
    ]
    files.sort(key=os.path.getmtime, reverse=True)
    got = []
    for i, f in enumerate(files):
        prompts, recalled = read_file(f)
        got += [(ts, cwd, text, recalled.get(uuid, [])) for ts, cwd, uuid, text in prompts]
        if len(got) >= n and i >= 1:
            break
    got.sort(key=lambda p: p[0])
    return got[-n:]


def slice_for(binary, cwd, host, cache):
    key = (cwd, host)
    if key not in cache:
        cmd = [binary, "--cwd", cwd, "--host", host]
        done = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        cache[key] = (done.stdout, done.stderr.strip())
    return cache[key]


def short(text, n):
    text = " ".join(KEY.sub("[redacted]", text).split())
    return text if len(text) <= n else text[: n - 1] + "…"


def tilde(path):
    home = os.path.expanduser("~")
    return "~" + path[len(home):] if path.startswith(home) else path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--n", type=int, default=30)
    ap.add_argument("--bin", default="target/debug/examples/outside_slice")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    prompts = last_prompts(args.n)
    cache = {}
    rows = []
    for ts, cwd, text, hits in prompts:
        cc, _ = slice_for(args.bin, cwd, "claude-code", cache)
        cx, _ = slice_for(args.bin, cwd, "codex", cache)
        rows.append((ts, cwd, text, hits, cc, cx))

    out = []
    w = out.append
    w("# Memory V1 vs V2 — the last %d Claude Code prompts\n" % len(rows))
    w("Offline, no model calls. **V1** = the \"From memory\" lines the plugin's per-turn recall put in front of Claude Code "
      "(already in the log). **V2** = what the forwarding hands a session in that folder, by the engine's own reader "
      "(`outside_memory::slice`, 4 KB budget), once at session start.\n")
    w("- *V2 for Claude Code* is empty by design: an agent is never forwarded its own memory, and Codex's memory format "
      "is not read yet. What V2 gives Claude Code is what Codex remembers — nothing yet.")
    w("- *V2 for Codex* is what a Codex session in the same folder would receive: Claude Code's memory for that project and the "
      "person's headings from `~/.claude/CLAUDE.md`. That is the cross-agent sharing V2 exists for.")
    w("- V1 recall is per turn and ranked by meaning; V2 is one index per session, chosen by folder and recency. "
      "Whether a V2 line would have helped this prompt is for you to judge below.\n")
    w("| # | when | folder | prompt | V1 hits | V1 chars | V2 → Claude Code | V2 → Codex |")
    w("|---|---|---|---|---|---|---|---|")
    for i, (ts, cwd, text, hits, cc, cx) in enumerate(rows, 1):
        w("| %d | %s | `%s` | %s | %d | %d | %d B | %d B |" % (
            i, ts[:16].replace("T", " "), tilde(cwd), short(text, 60).replace("|", "/"),
            len(hits), sum(len(h) for h in hits), len(cc.encode()), len(cx.encode())))
    w("\nV1 recall repeats every turn; V2's bytes are paid once per session.\n")

    w("## Prompt by prompt\n")
    for i, (ts, cwd, text, hits, cc, cx) in enumerate(rows, 1):
        w("### %d. %s — `%s`\n" % (i, ts[:16].replace("T", " "), tilde(cwd)))
        w("**Prompt:** %s\n" % short(text, 300))
        w("**V1 recalled (%d):**" % len(hits))
        for h in hits:
            w("- %s" % short(h, 260))
        if not hits:
            w("- (nothing)")
        w("\n**V2 → Claude Code:** %s" % ("(nothing — its own memory is not forwarded to it)" if not cc else "see the folder's slice below"))
        w("**V2 → Codex:** %s\n" % ("(nothing for this folder)" if not cx else "slice for `%s` (%d B) below" % (tilde(cwd), len(cx.encode()))))

    w("## V2 slices by folder, as a Codex session would receive them\n")
    for cwd in dict.fromkeys(r[1] for r in rows):
        block, notes = cache[(cwd, "codex")]
        w("### `%s`\n" % tilde(cwd))
        w("```\n%s\n```" % (block.rstrip() or "(nothing)"))
        if notes:
            w("\n_%s_" % notes.replace("\n", "; "))
        w("")

    path = os.path.join(args.out, "v1-vs-v2.md")
    with open(path, "w") as fh:
        fh.write("\n".join(out) + "\n")
    print(path, len(rows), "prompts,", len({r[1] for r in rows}), "folders")


if __name__ == "__main__":
    sys.exit(main())
