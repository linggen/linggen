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
cwd: ~/.linggen
# The engine itself runs the walk of the day's session logs before the
# first turn — the running `ling`, these arguments, no shell, no model.
# It writes ~/.linggen/memory/.scan-output.jsonl; if it fails the run ends
# failed, unstamped, before any model call. A trigger with no day fails
# here too.
prepare: [mem, scan, $DAY]
# One candidate session per turn, each turn starting clean: a day's
# transcripts run to 200 KB a session, and reading them all in one
# context would carry every earlier one into every later call.
kickoff-fresh: true
kickoff-stop: [DONE, CLEAN, FAILED]
kickoff-day:
  - >-
    You are in the memory scan, for one day: $DAY. The engine has
    already walked the day's session logs into
    `~/.linggen/memory/.scan-output.jsonl`. Introduce the run in one short
    line, then steps 1–3 of your scan procedure: Read line 1 (the
    header) — a missing file, or a header whose `window` is not $DAY →
    your `SCAN $DAY failed` line, then exactly: FAILED. Then, for each
    candidate line 2 … `sessions_scanned` + 1, Read only its opening
    (`line_range` [n, n], `max_bytes` 600: the session fields come before
    the transcript); and collect the day's covered sessions. Reply with
    `SCAN $DAY found=<n> scanned=<n>`, a `SKIP <session-id> covered` line
    per covered candidate, and `TODO <file line numbers of the candidates
    left>`. No candidates left → stamp the day per your procedure and
    reply with your status lines, then exactly: CLEAN. Otherwise stop
    and wait.
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
# Read the candidates, write rows through memory. No Bash: the run has no
# command to run — the engine ran the only one.
allowed-tools:
  - mcp__memory
  - Read
# Read on the one file the scan writes, and nothing else. No grant names
# the session folders or the rest of the Linggen home. Missions never
# prompt: a call outside this grant is refused.
permission:
  paths:
    - { path: ~/.linggen/memory/.scan-output.jsonl, mode: read }
  warning: >-
    Before the run, the engine runs `ling mem scan <day>`, which reads
    that day's session logs on this machine — ~/.claude/projects,
    ~/.codex/sessions, ~/.codex/archived_sessions, ~/.openclaw/agents and
    ~/.linggen/sessions — flattens them, drops secret-looking lines, and
    writes ~/.linggen/memory/.scan-output.jsonl; no network, no model.
    The agent then reads only that file (it has no shell) and writes
    short-term memory rows through this machine's own memory, each
    carrying the session it came from, and stamps the day scanned. It
    never promotes, merges or deletes rows.
---

# Memory scan — run protocol

Your scan procedure — the candidates file, which sessions are covered,
what is worth staging, how a row is written, the stamp — is in your
system prompt (you are the `memory` agent, "Procedure — scan one day").
This mission adds only how the run is paced:

- **Before turn 1** the engine has run the scan (`ling mem scan <day>`);
  you never run it.
- **Turn 1** checks the file's header against the run's day, lists the
  candidates and plans: `SCAN …`, the `SKIP … covered` lines, and the
  `TODO` list of candidate lines (file line numbers).
- **One candidate per turn** after that: read that one line, judge,
  write, `STAGE <line> <session-id> rows=<k>`. Each turn starts clean;
  your earlier replies stay, so the TODO and STAGE lines are the plan
  and the progress.
- **Done** when every TODO line has a STAGE line: stamp the day, reply
  `DONE`. Nothing to stage → stamp, `CLEAN`. A missing or stale file →
  no stamp, `FAILED`. Out of turns → no stamp,
  `PARTIAL <n> sessions remain`.
- **Report as you go.** The status lines are the whole surface. Never
  print one for a tool call you did not make.
