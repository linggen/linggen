---
name: Memory solve
description: Works through memory's review queue — the items the dream could not settle — from evidence first, asking the user one plain question only when evidence cannot settle an item. Started from Settings → Memory with the user watching; never on a schedule. Built-in.
# No schedule: this mission runs only when triggered —
# `POST /api/missions/memory-solve/trigger {"attended":true}` from the
# memory screen. `attended` puts AskUser in the run's tools; without it
# the run still solves what evidence settles and leaves the rest open.
# `enabled: false` keeps it off the scheduler; a trigger runs it all the
# same.
enabled: false
# The `memory` agent carries the solve runbook (agents/memory.md,
# "Procedure — solve the review queue"); this file carries the run
# protocol.
agent: memory
cwd: ~/.linggen
# A v1 memory job: under `[memory] linggen = "v2"` nobody reads the v1 store, so
# the run is skipped with a plain reason — no model call, no session
# (`ling v1-only`, `src/cli/v1_only.rs`, the one place that decides).
prepare: [v1-only]
kickoff-stop: [DONE]
kickoff:
  - >-
    You are in the memory solve. Introduce it in one short line, then
    work the whole review queue per your solve procedure — replica check
    first, then every open item, one at a time, evidence first. End with
    the report lines and the counts, then reply exactly: DONE.
allowed-tools:
  - mcp__memory
  - Read
  - Glob
  - Grep
  - Bash
# The same grant as the dream: READ over the home directory, so the
# evidence — a project's files, docs and git history — can be looked at
# (Read, grep, git -C <dir> log/show/diff/blame) with no prompt. Anything
# above read — a write, a git write, rm, a redirect — is refused at once
# (missions never prompt). The changes a solve makes go through memory's
# own tools.
permission:
  paths:
    - { path: "~", mode: read }
  warning: >-
    Talks to this machine's own memory only. For each open review item
    it reads the rows involved and checks them against files, docs and
    git history under your home directory (read only — anything that
    writes is refused), fixes the agent's own notes where the evidence
    settles it (a replace_ids merge archives the old rows, recoverable),
    and closes the item with a one-line note. A row in your own words
    changes only after you answer its question. The engine snapshots the
    store before each run.
---

# Memory solve — run protocol

Your solve runbook — evidence first, the confidence rule, how to ask,
closing items, the report — is in your system prompt (you are the
`memory` agent, "Procedure — solve the review queue"). This mission
adds only:

- **One run drains the queue.** Item by item, without pausing to report
  or to ask "continue?". If the context grows heavy, finish the item in
  hand, report the counts and how many remain, say the next solve
  continues, and reply `DONE`.
- **Asking.** AskUser is in your tools only when the user started this
  run and is watching. Without it, an item that needs the user stays
  open with its question in your report.
- **Report as you go.** The status lines are the whole surface. Never
  print one for a tool call you did not make.
