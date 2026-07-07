---
name: condense
description: Monthly long-term memory maintenance. Collapses stale same-subject chains in semantic memory into current-truth rows — the only pass that revisits semantic-at-rest. Built-in.
# Monthly, 4am on the 1st. SHIPS DISABLED: first runs are supervised —
# trigger manually from the memory app / API after taking a backup
# (`ling-mem export ~/condense-backup.ndjson`), watch the merges, then
# enable the cron once trusted.
schedule: "0 4 1 * *"
enabled: false
# No turn-seam catch-up: a missed month is fine, and an unattended
# catch-up would defeat the supervised rollout. Cron only, once enabled.
agent: memory
cwd: ~/.linggen
# Multi-item kickoff, same engine-driven loop as the dream mission:
# item 0 starts the run; each later item lands after the model's final
# reply. Phase 1 (cited chains) always fetches at offset 0 — merged
# chains vanish from the next scan, and `derived_only:true` keeps
# unmergeable user-voice clusters out of the page entirely, so the
# front of the list is always fresh work. Phase 2 (marker candidates)
# pages by fixed offsets; skipped candidates linger but re-paging past
# them is one cheap turn. A longer backlog continues next run.
kickoff:
  - >-
    You are in the condense mission — monthly maintenance of long-term
    memory. Introduce it in one short line, then call
    `Memory_query({"verb":"chains","kind":"cited","limit":3,"derived_only":true})`.
    If `total` is 0, reply exactly: CITED-CLEAN. Otherwise collapse
    each returned chain per the condense procedure in your system
    prompt (one current-truth row via `replace_ids`, never citing raw
    row ids in the new content), report a `MERGE` line per chain, then
    stop and wait.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"cited","limit":3,"derived_only":true})`
    for a FRESH scan — never reuse a previous turn's response. From
    ONLY that fresh result: `total` 0 → reply exactly: CITED-CLEAN.
    The first returned chain is one you already reported `MERGE` for
    this run → reply exactly: STALLED (your merge did not take;
    do not retry). Otherwise collapse each returned chain per your
    system prompt and report `MERGE` lines.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"cited","limit":3,"derived_only":true})`
    for a FRESH scan — never reuse a previous turn's response. From
    ONLY that fresh result: `total` 0 → reply exactly: CITED-CLEAN.
    The first returned chain is one you already reported `MERGE` for
    this run → reply exactly: STALLED (your merge did not take;
    do not retry). Otherwise collapse each returned chain per your
    system prompt and report `MERGE` lines.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"cited","limit":3,"derived_only":true})`
    for a FRESH scan — never reuse a previous turn's response. From
    ONLY that fresh result: `total` 0 → reply exactly: CITED-CLEAN.
    The first returned chain is one you already reported `MERGE` for
    this run → reply exactly: STALLED (your merge did not take;
    do not retry). Otherwise collapse each returned chain per your
    system prompt and report `MERGE` lines.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"cited","limit":3,"derived_only":true})`
    for a FRESH scan — never reuse a previous turn's response. From
    ONLY that fresh result: `total` 0 → reply exactly: CITED-CLEAN.
    The first returned chain is one you already reported `MERGE` for
    this run → reply exactly: STALLED (your merge did not take;
    do not retry). Otherwise collapse each returned chain per your
    system prompt and report `MERGE` lines.
  - >-
    Marker phase. First action this turn: call
    `Memory_query({"verb":"chains","kind":"marker","limit":5,"offset":0,"derived_only":true})`.
    For each candidate, CONFIRM before touching anything: the marker
    row and a neighbor are the same subject AND one completes or
    obsoletes the other. Confirmed → collapse per your system prompt
    (`replace_ids` may list only neighbor rows that are
    `from=derived, tier=semantic`). Not confirmed → `SKIP <id>
    unrelated`. No candidates → reply exactly: MARKER-CLEAN.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"marker","limit":5,"offset":5,"derived_only":true})`
    for a FRESH page. Same procedure: confirm each candidate, collapse
    confirmed ones, `SKIP <id> unrelated` for the rest. Empty page →
    reply exactly: MARKER-CLEAN.
  - >-
    First action this turn: call
    `Memory_query({"verb":"chains","kind":"marker","limit":5,"offset":10,"derived_only":true})`
    for a FRESH page. Same procedure: confirm each candidate, collapse
    confirmed ones, `SKIP <id> unrelated` for the rest. Empty page →
    reply exactly: MARKER-CLEAN.
  - >-
    Last turn for this run. Call
    `Memory_query({"verb":"chains","kind":"cited","limit":1,"derived_only":true})`
    for a fresh count, then reply exactly:
    `DONE merged=<your MERGE line count this run> remaining-cited=<total from the fresh call>`.
    Remaining chains continue next run — oldest-first keeps progress
    monotone.
# Unattended once enabled: no chat partner, no AskUser. The agent spec
# (agents/memory.md) carries the condense judgment doctrine — merge law,
# drafting rules, status lines; this body carries only the run protocol.
allowed-tools:
  - Memory_query
  - Memory_write
permission:
  warning: >-
    Talks to the local ling-mem daemon on 127.0.0.1 only. Merges the
    agent's own derived long-term rows via atomic replace_ids (insert
    one current-truth row, retire the cited chain members). Never
    touches user-voice rows, core rows, or episodic staging; touches
    no files directly.
---

# Memory condense — monthly run protocol

Your judgment doctrine — the merge law, how to draft a condensed row,
the status-line format — is in your system prompt (you are the
`memory` agent). This mission adds only the run protocol:

- **One page per turn.** Each turn: fetch a fresh chains page
  (`derived_only:true`, always), collapse what it returns, stop. The
  next kickoff nudge continues the loop. Cited chains re-fetch at
  offset 0 — merged chains vanish from the next scan; marker
  candidates page by fixed offsets.
- **Cited chains are pre-confirmed.** An id-citation edge is proof of
  reference — collapse without re-litigating. Marker candidates are
  guesses: confirm same-subject supersession against the neighbors
  before merging, `SKIP <id> unrelated` otherwise.
- **Stop conditions.** Fresh cited scan total 0 → `CITED-CLEAN`.
  First returned chain already merged this run → `STALLED` (a merge
  did not take; a human will look — do not loop). Empty marker page →
  `MARKER-CLEAN`. Final turn → `DONE merged=<n> remaining-cited=<m>`.
- **Failure = tool_error only.** A failed HTTP call / unreachable
  daemon → say `Condense failed: <short reason>` and stop. Everything
  else — `"action":"merged"` on add, an already-gone row — is normal.
- **Report as you go.** One `MERGE <new-id> replaces=<k> "<gist>"`
  line per collapsed chain; `SKIP <id> unrelated` per rejected marker
  candidate. Never print a status line for a tool call you did not
  make.
