---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge that travels with the user across sessions and
hosts — who they are, how they want to work, the decisions they've
made. Memory helps every kind of user, not just coders.

**Is:** identity, preferences, decisions with their reasoning, gotchas
the user re-hits. **Isn't:** an activity log (git records that), a
codebase snapshot (files are truth), a transcript (the session store
records that). A candidate that earns no place is dropped. Memory
never writes to project files.

## Parts

- **ling-mem** — standalone binary + local daemon; owns the store and
  all mechanical operations (dedup, search, export). No LLM inside;
  every frontend goes through it, so deleting a frontend never loses
  data.
- **Linggen engine** — memory tools for every agent, the always-on
  identity block in each session, per-turn recall, and the capture
  protocol in the system prompt.
- **Memory agent + missions** — the offline judgment brain; runs the
  nightly **dream** and monthly **condense** missions. The memory
  app's buttons trigger the same missions.
- **shared-memory skill** — the same store in Claude Code, Codex, and
  OpenClaw: recall each turn, capture protocol, runbooks, and the
  memory app UI (calendar, dashboard, row browser).

## Tiers

- **Core** — a handful of high-confidence universals about the person
  (name, role, hard work rules), present in every session.
- **Long-term** — everything else durable, retrieved on demand. Holds
  *state and lessons, never events*: would the row still matter in
  three months?
- **Short-term** — per-turn working capture. Events and uncertain
  signal land here; once the dream pass has judged a day, its rows
  fade after about a week unless promoted.

## Features

- **Capture** — the live agent saves signal in the same turn it
  appears, on every host. Explicit statements go straight to
  long-term; incidental signal stages in short-term.
- **Recall** — relevant memories surface at the start of each turn;
  facts used in a reply are cited ("From memory: …"). The identity
  set is always present.
- **Dedup** — exact duplicates collapse automatically at write time.
  Anything fuzzier is judgment, not mechanics.
- **Reconcile** — authority follows voice: the agent freely merges
  and rewrites *its own* notes into current truth; anything the
  *user* said changes only with the user (ask first). The store
  itself enforces the floor — a silent rewrite of the user's voice
  is refused, on every host. Whoever sees garbage fixes it in that
  moment — there is no cleanup queue.
- **Dream** (nightly) — reviews each day's short-term staging,
  promotes what's durable, and lets the rest fade. Never deletes
  unjudged rows. Day-by-day, with a visible per-day state on the
  calendar.
- **Scan** (user-triggered) — backfills a past day from host session
  logs, for days live capture missed. Safe to re-run: sessions that
  already contributed are skipped.
- **Condense** (monthly) — cures stale long-term memory: chains of
  superseded notes and clusters of same-subject notes collapse into
  one focused current-truth row each. Only touches the agent's own
  notes; ships off until supervised runs earn trust; back up first.
- **Secrets** — credentials never enter memory; deleted on sight.

## Rules

1. The user's voice changes only with the user — no silent rewrites
   of anything they said.
2. Record what was said; never invent details. Stamp ages to a date,
   not "now".
3. The file beats the memory — anything readable from the workspace
   stays out.
4. Curate, don't accumulate — value grows over time, not row count.
5. Merging the agent's notes is free; generalizing about the user is
   always done in front of them.
6. Never store secrets, at any layer.

## Evaluation

LongMemEval is a retrieval regression check only — it rewards
hoarding, the opposite of this design; never optimize toward it. The
real scorecard is the write-side eval (six axes: extraction, routing,
dedup, reconcile, decay, secrets), which drives each scenario through
a real engine against a throwaway store and judges the end state.

## Open / next

1. A dedicated inspectable/undoable widget for mission promotes and
   merges.

## Future

- Cross-device sync — exports + git first; real sync over Linggen's
  P2P transport.
- Soft-forget (archive): hidden from search but recoverable.

## Where the detail lives

Capture protocol: the engine system prompt + the skill's MCP
instructions · offline judgment: `agents/memory.md` · dream/condense
procedures: the shared-memory skill's runbooks · store schema and
CLI: `linggen-memory` docs · layout: `storage-spec.md` · tool
dispatch: `tool-spec.md`.
