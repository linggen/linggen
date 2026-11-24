---
name: Chronicle
description: Once a month, writes the months that have ended into Linggen's chronicle — what the person did and how far they came — from their own notes, sessions and what other agents learned; a past year gets its summary. Memory v2 only (`[memory] linggen = "v2"`); with v2 off, or nothing waiting, the run ends with no model call. `ling chronicle backfill` runs it now. Built-in.
# The 1st of each month, early. If the Mac was busy or off, the catch-up
# sweep runs it in the next quiet window (the Mac idle, not in the middle of a
# chat) within the month.
schedule: "0 4 1 * *"
catchup_hours: 720
enabled: true
# The `chronicler` agent writes only the chronicle, through Linggen's one
# note writer; the fence lets it write nothing else.
agent: chronicler
# The memory folder: a tool with no path of its own is judged where the run
# stands, so the one read grant below covers the input file and `memory_notes`.
cwd: ~/.linggen/memory
# The engine itself reads the sources before the first turn — the running
# `ling`, no shell, no model. It writes ~/.linggen/memory/.chronicle-input.jsonl
# and exits "nothing to do" (run ends quietly, no model call) when memory v2
# is off or no month is waiting.
prepare: [chronicle, prepare]
# One month (or year) per turn, each turn starting clean: a month's sources
# run to tens of KB, and reading them all in one context would carry every
# earlier month into every later call.
kickoff-fresh: true
kickoff-stop: [DONE, FAILED]
kickoff:
  - >-
    You are in the chronicle run. The engine has already written the
    sources to `~/.linggen/memory/.chronicle-input.jsonl`. Introduce the run
    in one short line. Read line 1 (the header; `line_range` [1, 1]): a
    missing file, or a line that is not a header → your line
    `CHRONICLE failed`, then exactly: FAILED. Otherwise reply
    `CHRONICLE months=<the header's months> years=<the header's years>`,
    then do the first item below in this same turn.
    Each later line of the file is one month (it has `month`) or one year
    (it has `year`). Take the lowest line number, from 2 on, that has no
    `WROTE` line yet: Read only that line (`line_range` [n, n],
    `max_bytes` 250000). For a month, write its dated lines as your ground
    rules say, from that line's `notes`, `sessions` and `other_agents`.
    For a year, Read each of its `month_files` with `memory_notes op=read
    path=…` and write the year's few most important lines (topic = the
    year). Reply `WROTE <line> <month or year> lines=<k>`, then stop and
    wait. Every line already has a `WROTE` line → reply exactly: DONE.
  - &next >-
    First check this run's `WROTE` lines. Take the lowest line number, from
    2 on, that has no `WROTE` line yet: Read only that line of
    `~/.linggen/memory/.chronicle-input.jsonl` (`line_range` [n, n],
    `max_bytes` 250000), write its lines as your ground rules say (a month
    from its `notes`, `sessions`, `other_agents`; a year from its
    `month_files`, read with `memory_notes op=read path=…`), reply
    `WROTE <line> <month or year> lines=<k>`, then stop and wait. Every line
    already has a `WROTE` line → reply exactly: DONE.
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
  - *next
  - >-
    Last turn for this run. Every line has a `WROTE` line → reply exactly:
    DONE. Otherwise reply exactly: `PARTIAL <n> lines remain` (the next run
    goes on with the months and years that have no file).
# Read the one input file, write the chronicle through memory_notes. No
# Bash, no other tool.
allowed-tools:
  - memory_notes
  - Read
permission:
  paths:
    - { path: ~/.linggen/memory, mode: read }
  warning: >-
    Before the run, the engine runs `ling chronicle prepare`, which reads
    this machine's Linggen notes, the sessions you spoke in (never a
    mission's, an app's, a host's or another person's), and the first lines
    of other agents' notes (Claude Code, Codex), drops secret-looking
    strings, and writes ~/.linggen/memory/.chronicle-input.jsonl; no
    network, no model. The agent then reads that file (and Linggen's own
    notes, ~/.linggen/memory) and writes dated lines into your chronicle
    (~/.linggen/memory/chronicle/), which you can edit or delete. It never changes another note, and never
    touches another agent's files. Deleting a month's file has it written
    again at the next run.
---

# Chronicle — run protocol

Your ground rules are in your system prompt (you are the `chronicler`). This
mission adds only how the run is paced:

- **Before turn 1** the engine has prepared the input (`ling chronicle
  prepare`); you never run it. With memory v2 off, or no month waiting, the
  run ended before any model call.
- **One month or year per turn** after the header: read that one line, write
  its lines, `WROTE <line> <period> lines=<k>`. Each turn starts clean; your
  earlier replies stay, so the `WROTE` lines are the progress.
- **Done** when every line has a `WROTE` line: `DONE`. A missing or odd file
  → `FAILED`. Out of turns → `PARTIAL <n> lines remain`.
