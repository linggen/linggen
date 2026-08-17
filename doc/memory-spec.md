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
- **Memory agent + mission** — the offline judgment brain; runs the
  nightly **dream** mission (remember → forget → condense). The memory
  app's buttons trigger the same mission.
- **linggen plugin/skill** (formerly shared-memory) — the same store in
  Claude Code, Codex, and OpenClaw: recall each turn, capture protocol,
  runbooks, and the memory app UI (calendar, dashboard, row browser).
  Third-party hosts reach the store through the daemon's `/mcp`
  `memory_*` group (`mcp-spec.md`).

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
- **Scope** — every row records the project directory it came from,
  and recall is scoped to the project the question is asked in (plus
  every row that belongs to no project — identity, preferences,
  cross-project gotchas are about the person). The host stamps both
  sides mechanically; the model is never asked to copy either. A
  directory that is not a project (home, the engine's own state dir,
  temp) never becomes a scope, and rows carried forward by the dream
  or a backfill keep the scope they were born with.
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
  calendar. Ends with the condense stage once the worklist is clear.
- **Scan** (user-triggered) — backfills a past day from host session
  logs, for days live capture missed. Safe to re-run: sessions that
  already contributed are skipped.
- **Audit** (dream's last stage) — cures stale long-term memory by
  confidence: what the agent can solve it solves, the rest is queued
  for the user. Two confident lanes, both capped per night, store
  snapshot before every run: condense — high-confidence cited chains
  of its own notes collapse into one current-truth row each — and
  completion-bar marker merges — a provisional-state note whose
  strictly newer same-subject derived neighbor asserts the work done
  collapses the same way (the store already holds the answer).
  Everything else (uncertain merges, status claims likely overtaken
  by the world, user-voice conflicts) becomes a **review item** in
  the daemon's issues queue — bookkeeping only, no row changes. The
  marker scan excludes rows a review item already names: queueing
  consumes nothing, so without that exclusion the capped page would
  re-serve the same candidates forever.

  Merges ARCHIVE, never delete (2026-08-17): a `replace_ids` loser in
  the semantic table gets `expired_at` + `superseded_by = <survivor>`
  — invisible to search, list, scans, and counts, kept on disk, so
  every merge and digest can be unpacked (`list --superseded-by
  <id>`; `--include-expired` widens any read). Episodic losers stay
  hard-deleted: staging is disposable. The stats surface reports the
  archive count — archived state is visible state.

  The third confident lane, **subject digests**, runs in the dream
  too: the `subject` detector (cosine star clusters, 3–12 members)
  serves only QUIET clusters (newest member >30 days — a live
  subject keeps its detail) and skips rows a `subject`-kind ruling
  covers. The judge digests clusters it is confident share one
  subject (one digest row, tagged `digest`, ≤5 per night) and queues
  doubtful ones as open `subject` review items — listing ALL member
  ids, so a ruled cluster can never re-form. In solve, the user's
  keep-separate answer becomes the permanent (dismissed) ruling.
  Digesting unattended is safe because merges archive: a wrong
  digest is an unpack, not a loss.
- **Solve** (attended) — a host agent drains the review queue with
  the user present: gathers evidence at solve time (git history,
  files), fixes what the evidence proves, asks the user one item at a
  time for the rest, closes each item. Surfaces: `/linggen solve` on
  plugin hosts, the memory app chat on Linggen; the ling-mem console
  shows the queue read-only. Dream reports and the recall footer
  carry the open count.
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
7. Status rows are perishable — a capture that changes a subject's
   status supersedes the prior status row in the same write; the
   review queue is the backstop, never the plan.

## Evaluation

LongMemEval is a retrieval regression check only — it rewards
hoarding, the opposite of this design; never optimize toward it. The
real scorecard is the write-side eval (six axes: extraction, routing,
dedup, reconcile, decay, secrets), which drives each scenario through
a real engine against a throwaway store and judges the end state.

## Open / next

- Scope on Codex is read-side only: its hook runner cannot rewrite
  tool input yet, so recall is scoped but the model's own writes go
  unstamped there.
- Rows from before scoping whose session logs are gone still carry no
  project and surface everywhere; stamping them by content is
  inference on the user's memory — their call, not the agent's.

## Future

- Cross-device sync — exports + git first; real sync over Linggen's
  P2P transport.
- Soft-forget (archive): hidden from search but recoverable.

## Where the detail lives

Capture protocol: the engine system prompt + the skill's MCP
instructions · offline judgment: `agents/memory.md` · dream/condense
procedures: the linggen skill's runbooks · store schema and
CLI: `linggen-memory` docs · layout: `storage-spec.md` · tool
dispatch: `tool-spec.md`.
