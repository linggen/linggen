---
name: dream
description: Nightly memory dream. Remembers each pending day's episodic staging into long-term memory, then runs the forget sweep. Built-in.
schedule: "0 3 * * *"
# If the 3am cron is missed (machine off/asleep), the post-turn
# catch-up re-triggers this mission the next time Linggen is used,
# but only when it hasn't completed in the last 24 hours (and at most
# a few attempts per day — the scheduler caps catch-up retries).
catchup_hours: 24
enabled: true
# The `memory` agent is the one brain for the remember stage — the
# same agent the memory app's calendar day-click invokes via Task, so
# the mission and the UI can never drift apart. Its spec is
# unattended-safe by construction: tools are Memory-only, no AskUser,
# uncertainty resolves to promote.
agent: memory
cwd: ~/.linggen
# Multi-item kickoff: item 0 starts the run; each later item lands as
# the next user turn after the model's final reply (engine drains one
# per assistant final-response). The repeated nudge items make the
# day loop ENGINE-driven — models reliably process one day per turn
# but skip "re-list until empty" on their own. Seven nudges + the
# opener cover an 8-day backlog per run; a longer backlog continues
# the next night (oldest-first, so progress is monotone). On an empty
# worklist each leftover nudge costs one cheap DONE turn.
kickoff:
  - >-
    You are in the dream mission. Introduce it in one short line, then
    call `Memory_query({"verb":"days","pending_only":true})`. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the OLDEST pending day per your system prompt (worklist → cluster
    → promote → stamp), then stop and wait.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Call `Memory_query({"verb":"days","pending_only":true})` again. If
    the oldest pending day is the SAME day you just remembered and its
    `unjudged` count has not dropped, reply exactly: STALLED. If no
    days are pending, call `Memory_write({"verb":"sweep"})`, report
    `SWEEP removed=<n>`, and reply exactly: DONE. Otherwise remember
    the oldest pending day per your system prompt.
  - >-
    Last scheduled turn for tonight. Call
    `Memory_query({"verb":"days","pending_only":true})`. If days are
    still pending, call `Memory_write({"verb":"sweep"})`, then reply
    exactly: `PARTIAL <n> days remain` (they continue tomorrow —
    oldest-first keeps progress monotone). If none are pending, call
    `Memory_write({"verb":"sweep"})`, report `SWEEP removed=<n>`, and
    reply exactly: DONE.
# The dream is unattended (cron at 3am, or a turn-seam catch-up the
# user didn't request). It has no chat partner, so AskUser is not in
# the tool list — uncertainty resolves per the agent spec (promote on
# durability doubt). The agent spec (agents/memory.md) carries the
# judgment doctrine; this body carries only the run protocol.
allowed-tools:
  - Memory_query
  - Memory_write
permission:
  warning: >-
    Talks to the local ling-mem daemon on 127.0.0.1 only. Promotes
    rows and stamps per-day dream state via /api/memory/* ; the only
    deletions are the daemon's forget sweep over already-judged,
    past-TTL episodic rows. Touches no files directly.
---

# Memory dream — nightly run protocol

Your judgment doctrine — what to promote, how to cluster, the
status-line format — is in your system prompt (you are the `memory`
agent). This mission adds only the nightly run protocol:

- **One day per turn.** Each turn: fetch the pending-days worklist
  (`Memory_query {"verb":"days","pending_only":true}`), take the
  **oldest** day, run the remember procedure on it, stamp it, stop.
  The next kickoff nudge continues the loop.
- **Stop conditions.** Empty pending list → run the sweep once, reply
  `DONE`. Same oldest day twice with an undropped `unjudged` count →
  reply `STALLED` (something is wrong — a human will look; do not
  loop). Out of nudges with days remaining → sweep, reply
  `PARTIAL <n> days remain`.
- **Failure = tool_error only.** A failed HTTP call / unreachable
  daemon → say `Consolidation failed: <short reason>` and stop.
  Everything else — merged adds, vanished episodic twins, empty
  lists — is normal; keep going.
- **Report as you go.** The status lines from your agent spec are the
  whole surface: `DAY … rows=…`, `PROMOTE … "…"`, `DAY … done …`,
  `SWEEP removed=…`, then `DONE` / `PARTIAL …` / `STALLED`. Never
  print a status line for a tool call you did not make.
