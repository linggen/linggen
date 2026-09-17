---
name: dream
description: Nightly memory dream. Remembers each undreamed day's episodic staging into long-term memory, runs the forget sweep, then audits long-term memory — merging what it can prove (cited chains, completion-bar markers), digesting quiet subjects, and queueing what needs the user as review items. Built-in.
schedule: "0 3 * * *"
# If the 3am cron is missed (machine off/asleep), the post-turn
# catch-up re-triggers this mission the next time Linggen is used,
# but only when it hasn't completed in the last 24 hours (and at most
# a few attempts per day — the scheduler caps catch-up retries).
catchup_hours: 24
enabled: true
# The `memory` agent is the one brain for the remember stage — the
# same brain the memory app's calendar reaches by triggering this
# mission day-scoped (`kickoff-day` / `kickoff-attended` below), so
# the mission and the UI can never drift apart. Unattended runs are
# safe by construction: this mission's allowed-tools is memory-only
# (the engine adds AskUser to scope only on attended triggers) and
# uncertainty resolves to promote.
agent: memory
cwd: ~/.linggen
# Multi-item kickoff: item 0 starts the run; each later item lands as
# the next user turn after the model's final reply (engine drains one
# per assistant final-response). The repeated nudge items make the
# day loop ENGINE-driven — models reliably process one day per turn
# but skip "re-list until empty" on their own. Seven nudges + the
# opener cover an 8-day backlog per run; a longer backlog continues
# the next night (oldest-first, so progress is monotone).
# kickoff-stop: a reply ending on DONE (finish-up complete) or STALLED
# (a day already stamped this run came back — abort) ends the run; the
# engine discards the leftover items instead of burning a no-op turn on each.
kickoff-stop: [DONE, STALLED]
# kickoff-fresh: each turn starts clean — the finished turn's lists, searches
# and adds leave the context; its opening item and status reply stay, so the
# stall rule can still see this run's `DAY … done` lines. One run holding five
# days of worklists cost 2.08M prompt tokens (2026-09-17).
kickoff-fresh: true
# kickoff-then: a reply ending on CLEAR (the worklist is empty) swaps the
# leftover day nudges for the finish-up, one stage per turn — each stage's
# scan (the marker and subject scans run ~50KB and ~40KB) folds away before
# the next instead of riding every later call.
kickoff-then:
  CLEAR:
    - >-
      Finish-up 1 of 3 — the worklist is clear. Call `memory_sweep()`
      and report `SWEEP removed=<n>`. Then the cited-chains condense
      per your system prompt: ONE
      `memory_chains({"kind":"cited","limit":10,"derived_only":true})`
      fetch, collapse each returned chain (a `MERGE` line each; empty
      scan → no lines). End with `CONDENSE merged=<k>`, then stop and
      wait.
    - >-
      Finish-up 2 of 3 — the marker audit per your system prompt: ONE
      `memory_chains({"kind":"marker","limit":5})` fetch; merge each
      candidate that clears the completion bar (a `MERGE` line), queue
      the rest via `memory_issue_add({...})` (a `QUEUE` line). End with
      `MARKERS merged=<k> queued=<q>`, then stop and wait.
    - >-
      Finish-up 3 of 3 — the quiet-subject digest per your system
      prompt: ONE
      `memory_chains({"kind":"subject","limit":5,"derived_only":true})`
      fetch; digest each cluster you are confident is one subject (a
      `MERGE` line), queue the doubtful ones listing ALL member ids (a
      `QUEUE` line). Then reply exactly: DONE.
kickoff:
  - >-
    You are in the dream mission. Introduce it in one short line, then
    call `memory_days({"undreamed_only":true})`. No days undreamed →
    reply exactly: CLEAR (the finish-up follows as its own turns).
    Otherwise remember the OLDEST undreamed day per your system prompt
    (worklist → cluster → promote → stamp), then stop and wait.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    First action this turn: call
    `memory_days({"undreamed_only":true})` to fetch a FRESH worklist —
    never answer from a previous turn's response. Then decide from ONLY
    that fresh result: empty list → reply exactly: CLEAR. The oldest
    listed day already has a `DAY <date> done` line from you in THIS
    run → reply exactly: STALLED. A `remembered_at` from before this
    run is normal (late rows re-open a day) → remember it. Otherwise →
    remember the oldest listed day per your system prompt.
  - >-
    Last scheduled turn for tonight. First call
    `memory_days({"undreamed_only":true})` for a fresh count. From the
    fresh result only: no undreamed days → reply exactly: CLEAR. Days
    remain → call `memory_sweep()`, report `SWEEP removed=<n>`, and
    reply exactly: `PARTIAL <n> days remain` with n from the fresh
    response (they continue tomorrow — oldest-first keeps progress
    monotone; the finish-up waits for a night with a clear worklist).
# Day-scoped variant: used when a trigger passes a target day (the
# memory app's calendar dream button). $DAY is replaced by the engine
# with the YYYY-MM-DD date. Same procedure, one day, then the finish-up
# one stage per turn.
kickoff-day:
  - >-
    You are in the dream mission, scoped to a single day: $DAY.
    Introduce it in one short line, then run the remember procedure
    for $DAY per your system prompt (context → day worklist → cluster
    → promote → stamp via
    `memory_remember_day({"date":"$DAY", ...})` with the
    judged/promoted counts). If the day has no episodic rows, reply
    exactly: CLEAN. Then stop and wait.
  - >-
    Call `memory_sweep()` and report `SWEEP removed=<n>`. Then the
    cited-chains condense per your system prompt (`MERGE` lines; empty
    scan → no lines). End with `CONDENSE merged=<k>`, then stop and
    wait.
  - >-
    The marker audit per your system prompt: ONE
    `memory_chains({"kind":"marker","limit":5})` fetch (`MERGE` /
    `QUEUE` lines). End with `MARKERS merged=<k> queued=<q>`, then stop
    and wait.
  - >-
    Last turn for this run: the quiet-subject digest per your system
    prompt — ONE
    `memory_chains({"kind":"subject","limit":5,"derived_only":true})`
    fetch (`MERGE` / `QUEUE` lines). Then reply exactly: DONE.
# Attended day-scoped variant: the calendar day-click sends
# `attended: true` — the user just clicked and is watching, so the
# engine puts AskUser in scope and this kickoff ends with the
# low-confidence marker review the unattended runs must never do.
kickoff-attended:
  - >-
    You are in the dream mission, scoped to a single day: $DAY — an
    ATTENDED run: the user is watching and AskUser is available for
    the final review step only. Introduce it in one short line, then
    run the remember procedure for $DAY per your system prompt
    (context → day worklist → cluster → promote → stamp via
    `memory_remember_day({"date":"$DAY", ...})` with the
    judged/promoted counts). If the day has no episodic rows, reply
    exactly: CLEAN. Then stop and wait.
  - >-
    Call `memory_sweep()` and report `SWEEP removed=<n>`. Then the
    cited-chains condense per your system prompt (`MERGE` lines). End
    with `CONDENSE merged=<k>`, then stop and wait.
  - >-
    Last turn for this run: the attended review per your system prompt.
    Fetch `memory_chains({"kind":"marker","limit":4,"derived_only":true})`
    and, if candidates return, confirm them with the user in ONE
    AskUser call — merge only what they approve; on timeout or error
    queue the candidates instead (`issue_add`, `QUEUE` lines). Then
    reply exactly: DONE.
# The dream is unattended (cron at 3am, or a turn-seam catch-up the
# user didn't request). It has no chat partner, so AskUser is not in
# the tool list — uncertainty resolves per the agent spec (promote on
# durability doubt). The agent spec (agents/memory.md) carries the
# judgment doctrine; this body carries only the run protocol.
# The memory server by name — expanded to the tools it advertises, so a
# ling-mem that gains a verb does not need this list re-edited.
allowed-tools:
  - mcp__memory
permission:
  warning: >-
    Talks to the local ling-mem daemon on 127.0.0.1 only. Promotes
    rows and stamps per-day dream state via /api/memory/* ; the only
    deletions are the daemon's forget sweep over already-judged,
    past-TTL episodic rows. Atomic replace_ids merges of the agent's
    own derived long-term rows (high-confidence cited chains ≤10 per
    night, ≤5 marker candidates whose completion is already asserted
    by a newer note, and ≤5 subject digests over clusters quiet for
    30+ days) ARCHIVE their losers — expired + superseded_by,
    recoverable — never delete them; the engine also
    snapshots the store before each run. What it cannot solve with
    confidence it queues as review items (a JSON sidecar entry, no
    row changes) for the user to solve later. Touches no files
    directly.
---

# Memory dream — nightly run protocol

Your judgment doctrine — what to promote, how to cluster, the
status-line format — is in your system prompt (you are the `memory`
agent). This mission adds only the nightly run protocol:

- **One day per turn.** Each turn: fetch the undreamed-days worklist
  (`memory_days({"undreamed_only":true})`), take the
  **oldest** day, run the remember procedure on it, stamp it, stop.
  The next kickoff nudge continues the loop.
- **Stop conditions.** Empty worklist → reply `CLEAR`; the engine
  then runs the finish-up (below) as its own turns. The oldest day
  already has your `DAY <date> done` line from this run → reply
  `STALLED` (its stamp didn't take — a human will look; do not loop).
  A day dreamed on an earlier night that late rows re-opened is not a
  stall. Out of nudges with days remaining → sweep, reply
  `PARTIAL <n> days remain` (no finish-up on PARTIAL nights).
- **Finish-up = three turns, one stage each.** Each turn starts clean
  and fetches ONE capped scan: (1) sweep, then
  `memory_chains({"kind":"cited","limit":10,"derived_only":true})`
  — collapse each chain per your condense doctrine, one current-truth
  row via `replace_ids`, a `MERGE` line each, end with
  `CONDENSE merged=<k>`; (2) `memory_chains({"kind":"marker","limit":5})`
  — MERGE when the completion bar is met (a strictly newer
  same-subject derived neighbor asserts the marked work done),
  `issue_add` (a `QUEUE` line) for what you cannot solve with
  confidence, end with `MARKERS merged=<k> queued=<q>`; (3)
  `memory_chains({"kind":"subject","limit":5,"derived_only":true})`
  — the digest stage: collapse each quiet cluster you are confident
  shares one subject into a single digest row (a `MERGE` line each);
  queue doubtful clusters as `subject` issues listing ALL member ids
  (a `QUEUE` line each); reply `DONE`. The capped fetches are the
  nightly budget; leftovers wait for tomorrow. Every audit merge
  archives its losers (recoverable) — nothing in the audit deletes.
- **A failed save doesn't end the day.** A `tool_error` on a write may
  still have landed (a timeout says so): search its gist once, retry
  once if it's absent, then carry on and stamp the day. Only a daemon
  that answers nothing at all → say
  `Consolidation failed: <short reason>` and stop. Everything else —
  merged adds, vanished episodic twins, empty lists — is normal; keep
  going.
- **Report as you go.** The status lines from your agent spec are the
  whole surface: `DAY … rows=…`, `PROMOTE … "…"`, `DAY … done …`,
  `CLEAR`, `SWEEP removed=…`, `CONDENSE …`, `MARKERS …`, then `DONE` /
  `PARTIAL …` / `STALLED`. Never
  print a status line for a tool call you did not make.
