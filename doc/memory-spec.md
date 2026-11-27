---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Linggen memory

> **How to keep this spec**
> - It records WHAT each memory function does and the rules it obeys — no history, no code, no function signatures.
> - Detail lives where it executes; point there (last section), do not copy it.
> - A change in behaviour updates the matching line here in the same commit.
> - A claim you cannot find in the code does not belong here. Stay under ~150 lines.

Persistent knowledge that travels with the user across sessions and hosts:
who they are, how they work, decisions with their reasons, gotchas they re-hit.
**Isn't:** an activity log (git), a codebase snapshot (the files), a transcript
(the session store). Memory never writes to project files.

## Tiers

Rule for every host: semantic/core hold only what lasts; anything time-bound (go-ahead, progress, state, price) is written with no tier (episodic), and `replace_ids` keeps the losers' highest tier, so moving down needs `tier: "episodic"`.

- **Core** — a few high-confidence universals about the person (name, role,
  place, family). Always loaded; never scoped, never a work rule. ≤ 600 chars.
- **Semantic** — durable state and lessons, never events: the user's own words
  (preferences, decisions with why), measured results, gotchas. One conclusion
  + a path per row, ≤ 600 chars. Retrieved on demand.
- **Episodic** — per-turn staging and the record of work: progress, status,
  what was built / fixed / solved / tried. Short-lived (see Expiry).

## Write

- Capture happens in the turn the signal appears, on every host.
- **Semantic or core:** search the subject first; if a row already says it,
  update it (`replace_ids`) instead of adding. **Episodic:** no search needed.
- Episodic may hold built / fixed / solved / tried. A log row (type
  built/fixed/tried, not from the user) never leaves episodic — not by the
  dream, not by any path. A lasting lesson from it is a new short `learned`
  row (one conclusion + a path).
- The store's tier lock enforces this on every add path (MCP, REST, CLI): log
  types are placed in episodic whatever tier was asked; a semantic/core row
  over 600 chars is refused; `from=user` rows are never moved or refused.
- Exact duplicates collapse at write time. Credentials are never stored.
- A status row supersedes the prior status row in the same write.

## Read

- **Session start:** core rows plus the scope index (below) are loaded into the
  prompt.
- **Per turn:** relevant rows are recalled into the turn (engine; plugin hook
  on Claude Code / Codex). Recall never holds a prompt: while the embedding
  model warms, automatic recall skips the turn quietly.
- **Cite:** a fact used in a reply is cited ("From memory: …").
- **Fix on read:** a recalled row that disagrees with the user's turn, with
  what you see now (files, git, a tool result) or with another row is wrong.
  Fix it that turn and say so in one line. Own rows (derived/agent) are
  replaced freely (`memory_add` + `replace_ids`); the user's own words change
  only after asking them.

## Scope & index

- A row's scope is the directory it is *about*; none = about the person. The
  host stamps `cwd` and `root`; the model may name another listed scope.
- Recall sees rows under the session root, at its parents, and about the
  person. A skill's own session sees only its rows. Home, `~/.linggen` and temp
  see the person's rows plus at most two strong matches filed under a directory.
- The day's session (Ling and Yinyue, `sess-day-…`) recalls as if at `$HOME`:
  every scope, top-k by relevance (`recall_scope_root`, `runtime.rs`). Project
  sessions keep the scoping above.
- Each session shows what it started with — core rows and index — as a folded
  chip (`memory-start` row, never read back to the model), again only when the
  block changes; each turn's recall is a folded `memory-recall` chip.
- Standing rules carry `indexed` and a one-line summary; each session loads the
  summaries of its directory and parents, nearest first, within 3000 chars, and
  reads a row in full when its summary bears on the task.
- The dream fixes scope, index and summary itself when sure (≤ 10 per run,
  never content); unsure → left alone.

## Dream (nightly, memory agent)

- Judges each day's episodic rows, one day at a time, with a visible per-day
  state. Promotes what is durable (as a new semantic/core row in the tier
  rules above); never promotes a log row.
- **Never deletes a row of a day it has not judged.**
- **Audit / condense** (last stage, capped per night, store snapshot first):
  collapses cited chains of its own notes, and provisional-state notes already
  asserted done by a newer one, into one current-truth row; digests quiet
  same-subject clusters. Merged semantic losers are archived (`superseded_by`),
  not deleted; episodic losers are deleted.
- Anything else it settles from evidence (rows, files, git; read-only) and
  records as a closed review item. It queues an open item only when a user-voice
  row would change, the user must choose, or the evidence is out of reach.
- Runs only on the primary's engine, once a day.

## Expiry (episodic) — built 2026-11-26

- An episodic row lives 30 days by default; the user sets it on the settings page.
- Each row has **heat** (times it was actually *used* — cited in a reply; being
  pushed by recall without use does not count) and an updated-at time.
- The sweep: a row past its life (now − updated-at > TTL) with heat 0 is
  deleted; with heat > 0, heat resets to 0 and updated-at to now (one more period).
- Rows of a day the dream has not judged are never deleted.
- Heat is used ONLY here — not for recall ranking, not for the prompt, not for
  promotion to semantic.

## Scan

- Backfills one past day from the session logs of Claude Code, Codex, OpenClaw
  and Linggen, for days live capture missed. Started by the user; works on
  every machine.
- The engine reads and secret-filters the logs (no model); the agent judges one
  session per turn, skips sessions that already have rows for the day, and
  stages worth-keeping items as episodic rows carrying session id and directory.
- It never promotes (the next dream judges). It stamps the day scanned, also
  when nothing was staged; a missing/stale input ends failed, unstamped; too
  many sessions for one run ends partial, and scanning again continues.
- Other people's Linggen sessions are skipped.

## Solve

- Attended path for what the dream could not settle: works the open review
  items from evidence, asks the user one plain question at a time with a
  recommendation for the rest, closes each item. A user-voice row changes only
  after their answer. Runs on the primary. Started from Settings → Memory or
  `/linggen:solve`.
- Scan, solve and dream are separate missions; one of each may run at once.
  Missions never prompt: a call outside their grants is refused.

## Hosting & who reaches memory

- Since 2.3 the engine hosts the store (one writer, one port, `/mem/*` and the
  `/mcp` door; `ling mem` CLI). There is no separate daemon.
- Caller decides whose rows: this machine and approved `machine` devices see
  all the owner's memory; a `phone` sees only its person's rows and a limited
  verb set; no token, nothing. Memory enforces the person on every write.
- A call that embeds waits for model warm-up up to 20 s, then is refused
  (neither applied nor queued); calls that do not embed never wait.

## Across machines

- Stars: one primary, any number of replicas; the setting is in Settings →
  Memory. Reads come from the local copy; writes go to the primary, queued
  while it is away; the primary wins a conflict and files a review item.
- A replica joins by asking; Allow/Deny happens only on the primary's machine.
- Dream and solve run only on the primary; inline fixes and scan run anywhere.

## Rules

1. The user's voice changes only with the user — no silent rewrites.
2. Record what was said; never invent. Stamp dates, not "now".
3. The file beats the memory — what the workspace shows stays out.
4. Curate, don't accumulate.
5. Merging own notes is free; generalizing about the user is done in front of them.
6. Never store secrets.
7. A status row supersedes the prior status row in the same write.

## Evaluation

LongMemEval is a retrieval regression check only; it rewards hoarding — never
optimize toward it. The real scorecard is the write-side eval (extraction,
routing, dedup, reconcile, decay, secrets) against a throwaway store.

## Open / next

- Scope on Codex is read-side only: its hook cannot stamp the model's writes.
- Multi-primary sync (when no machine can be primary); the phone's half of
  sync is in `phone-memory-spec.md`.

## Where the detail lives

Tier lock, heat, scope, index, recall: `crates/linggen-memory/src/` ·
tools and doors: `doc/mcp-spec.md`, `src/server/mcp/` · hooks and runbooks:
`plugins/linggen/` · dream / scan / solve: `missions/`, `agents/memory.md` ·
scan CLI: `crates/linggen-memory/src/cli/` · layout: `storage-spec.md` ·
phone and account stamp: `phone-memory-spec.md` · caller rules:
`permission-spec.md`.
