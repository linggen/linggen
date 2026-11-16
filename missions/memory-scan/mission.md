---
name: Memory scan
description: Backfills one day into short-term memory from this machine's session logs (Claude Code, Codex, OpenClaw, Linggen), for the sessions live capture missed, then stamps the day scanned. Started from Settings → Memory, one day at a time; never on a schedule. Built-in.
# No schedule: this mission runs only when triggered with a day —
# `POST /api/missions/memory-scan/trigger {"day":"YYYY-MM-DD","attended":true}`
# from the memory calendar. `enabled: false` keeps it off the scheduler;
# a trigger runs it all the same.
enabled: false
# The `memory` agent carries the scan procedure (agents/memory.md,
# "Procedure — scan one day"); this file carries the run protocol.
agent: memory
# The run's own working folder, empty, under the mission's folder (the
# engine makes it). `ling mem scan` is an admin-class command with no
# path the gate can see, so it is judged here. The admin grant below
# names this folder only; like any admin grant it is not a shell sandbox
# (a command naming no path is judged here too).
cwd: ~/.linggen/missions/memory-scan/work
# One candidate session per turn, each turn starting clean: a day's
# transcripts run to 200 KB a session, and reading them all in one
# context would carry every earlier one into every later call.
kickoff-fresh: true
kickoff-stop: [DONE, CLEAN, FAILED]
# Without a day there is nothing to scan.
kickoff:
  - >-
    This scan run was started without a day, and a scan covers exactly
    one day. Reply exactly: NO DAY
kickoff-day:
  - >-
    You are in the memory scan, for one day: $DAY. Introduce it in one
    short line. Then steps 1–3 of your scan procedure: run
    `ling mem scan $DAY` (that command alone in the call); list the
    candidates without their transcripts —
    `jq -c '{source, filepath, user_turns, transcript_bytes}' ~/.linggen/memory/.scan-output.jsonl`
    (output line n is file line n; line 1 is the header); collect the
    day's covered sessions. Reply with `SCAN $DAY found=<n> scanned=<n>`,
    a `SKIP <session-id> covered` line per covered candidate, and
    `TODO <file line numbers of the candidates left>`. The scan failed →
    your `SCAN $DAY failed` line, then exactly: FAILED (no stamp). No
    candidates left → stamp the day per your procedure and reply with
    your status lines, then exactly: CLEAN. Otherwise stop and wait.
  - &next >-
    First check your `TODO` line and this run's `STAGE` lines. Take the
    lowest TODO line that has no STAGE line yet: Read only that line of
    `~/.linggen/memory/.scan-output.jsonl` (`line_range` [n, n],
    `max_bytes` 250000), judge it per your scan procedure, write its
    rows, reply `STAGE <line> <session-id> rows=<k>`, then stop and
    wait. Every TODO line already has a STAGE line → stamp the day per
    your procedure and reply with your status lines, then exactly: DONE.
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - *next
  - >-
    Last turn for this run. Every TODO line has a STAGE line → stamp the
    day per your procedure and reply exactly: DONE. Otherwise do not
    stamp; reply exactly: `PARTIAL <n> sessions remain` (scanning the
    day again continues — staged sessions are skipped).
allowed-tools:
  - mcp__memory
  - Read
  - Bash
# Exactly what the run touches, as the gate sees it:
# - admin on the run's own empty folder — `ling mem scan <day>` is judged
#   there (an unknown program is admin-class);
# - read on the one file the scan writes, the candidates.
# No grant names the session folders: `ling mem scan` reads them itself,
# and a Read, grep or cat of a session log is refused. Missions never
# prompt: a call outside these grants is refused.
permission:
  paths:
    - { path: ~/.linggen/missions/memory-scan/work, mode: admin }
    - { path: ~/.linggen/memory/.scan-output.jsonl, mode: read }
  warning: >-
    Runs `ling mem scan <day>`, which reads that day's session logs on
    this machine — ~/.claude/projects, ~/.codex/sessions,
    ~/.codex/archived_sessions, ~/.openclaw/agents and
    ~/.linggen/sessions — flattens them, drops secret-looking lines, and
    writes ~/.linggen/memory/.scan-output.jsonl; no network. The agent
    then reads that file and writes short-term memory rows through this
    machine's own memory (each carrying the session it came from), and
    stamps the day scanned. It never promotes, merges or deletes rows. No
    grant names the session folders, so the agent's own reads of them are
    refused. The admin grant names only the run's own empty folder; like
    any admin grant it is not a shell sandbox — a command naming no path
    is judged there.
---

# Memory scan — run protocol

Your scan procedure — the command, which sessions are covered, what is
worth staging, how a row is written, the stamp — is in your system
prompt (you are the `memory` agent, "Procedure — scan one day"). This
mission adds only how the run is paced:

- **Turn 1** runs the scan and plans: `SCAN …`, the `SKIP … covered`
  lines, and the `TODO` list of candidate lines (file line numbers).
- **One candidate per turn** after that: read that one line, judge,
  write, `STAGE <line> <session-id> rows=<k>`. Each turn starts clean;
  your earlier replies stay, so the TODO and STAGE lines are the plan
  and the progress.
- **Done** when every TODO line has a STAGE line: stamp the day, reply
  `DONE`. Nothing to stage → stamp, `CLEAN`. Out of turns → no stamp,
  `PARTIAL <n> sessions remain`.
- **Report as you go.** The status lines are the whole surface. Never
  print one for a tool call you did not make.
