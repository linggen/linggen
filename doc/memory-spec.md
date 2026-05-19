---
type: spec
reader: Coding agent and users
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge that travels with the user across sessions — about
who they are, how they want to work, the decisions they've made, and the
people and projects in their life. Memory must help every kind of user
(software engineer, musician, language learner, cook), not just coders.

> **Status: rebuild.** §1–§2 are the new architecture (hippocampus:
> capture → consolidate → recall; two LanceDB tables, no markdown).
> Decay and §3 rule 5 are resolved. Open items remain in "Open / next".
> No back-compatibility with the prior append-mostly single-store design.

## 1. What is memory

Memory is the **user's biography across sessions**, not the agent's
notebook of what it did.

**Is:** identity, role, life context; how the user wants the agent to
work; decisions and their reasoning; cross-project gotchas they re-hit.

**Isn't:** an activity log (git records that), a codebase snapshot (files
are truth), a transcript (the session store records that).

Knowledge lives in one of three scopes, picked by *how the agent needs
to see it*:

- **Core** — universals about the person (name, role, languages, hard
  work rules). Rows tagged `tier=core` in the semantic table, injected
  **unconditionally** into every system prompt (no similarity gate).
  Tiny. No markdown files — one substrate, no two-places sync.
- **Semantic** — everything else durable. Curated, deduplicated,
  similarity-retrieved on demand. The live recall hot path.
- **Episodic** — recent extracted experience, awaiting consolidation.
  Decays. Not on the live query path; it is the consolidator's input.

If a candidate earns none of the three, **drop it.** Memory never writes
to project files (`AGENTS.md`, `CLAUDE.md`, source, docs) — those are
user-curated; the agent reads them directly when needed.

## 2. Architecture — capture, consolidate, recall

A two-engine pipeline over three stores. The model is the hippocampus:
fast broad capture during waking, selective consolidation offline,
forgetting of what didn't earn permanence.

### Stores — two LanceDB tables, no markdown

- **Semantic table** — durable, curated, superseded in place. Rows
  carry a `tier`: `core` (always injected, no similarity gate) or
  `semantic` (similarity-retrieved on demand). Core and semantic share
  one table because they share a churn/volume profile (low-churn,
  durable, retrieval-relevant) and core is tiny — the
  separate-table rule applies only when profiles differ.
- **Episodic table** — high-volume, churny, decays. Encoder writes here.

Episodic is a **separate table** from semantic: it is high-churn and
high-volume; one shared ANN index would let episodic dominate it,
fragment it under churn, and dilute semantic relevance. ("Store" is
ling-mem's wrapper term; LanceDB calls it a table.)

Core was previously markdown files (`identity.md`/`style.md`); the
rebuild folds them into `tier=core` rows. Determinism comes from the
unconditional query, not the file. Gains: one substrate, core rows are
also vector-searchable, consolidator promotes by flipping the tier.
Loss: no external text-editor edit — mitigated by the memory UI/CLI.
Third-party hosts read core via the binary (`ling-mem list --tier
core`); no markdown inject path.

### Engines (the hippocampus)

One async subagent (`ling-mem`), fired **every N turns** (N default ~10,
configurable in Settings). **Per-session counter** — resets each
session; no startup trigger; sessions shorter than N turns are not
consolidated. This is a deliberate simplicity tradeoff (no rolling
cross-session counter), accepted over completeness.

At each fire, strictly ordered:

1. **Encode** — read the recent in-session exchange, apply §4's
   *exclusion* filters (drop file-derivable, secrets, pure activity)
   **plus a write-time usefulness bar** (write only what a future task
   benefits from; drop garbage), then write/dedup into the **episodic**
   table. The encoder is the *first* gate — episodic is recall-visible
   immediately now that recall spans both tables — and the consolidator
   remains the terminal promote/delete gate past-TTL. ≈ waking encoding.
2. **Consolidate** — process **past-TTL rows only**; each gets one
   terminal decision: promote worthy rows → semantic (extract +
   optional `supersedes` link, never a destructive rewrite) or delete.
   Generalizing / merging stay live-only (§3). May propose **semantic →
   `tier=core`** when a stable universal emerges (user-confirmed; rule
   1). Re-entrant by construction — a handled row leaves episodic, so
   each tick sees only unhandled rows. ≈ reflective consolidation while
   awake (not a nightly batch).
3. **Evict** — delete episodic rows older than `EPISODIC_TTL` by
   last-edit time (a touch resets retention).

`EPISODIC_TTL` default **7 days** (configurable in Settings). Both it
and the trigger interval must be > 0; losslessness comes from the
consolidate-before-evict ordering below, not a TTL/interval ratio (the
two are different units — wall-clock days vs. turns).

**Consolidate-before-evict resolves the keep-vs-delete question by
ordering, not policy:** a past-TTL row always gets a final promotion
pass in the same tick before it can be evicted. Episodic is therefore
both bounded and lossless except for rows the consolidator judged not
worth keeping — which is correct.

The subagent runs **async** (never blocks the user's turn). A transient
"running" widget shows while it works. A persistent result line lands
**only on a material change** (≥1 fact promoted or superseded) — no-op
ticks stay silent — and is inspectable / undoable, with an explicit
failure state if the subagent errors. All such copy obeys the
≤20-word user-facing brevity rule (`code-style.md`).

### Binary vs judgment split

- The **`ling-mem` binary** owns both tables + the mechanical primitives
  (embed, dedup, supersede, decay). No LLM in the binary.
- **Judgment** ("what's worth keeping") needs an LLM, so it is an agent,
  scheduled per host:
  - **Linggen** — built-in: engine schedules the encoder subagent and
    the consolidator cron mission. Reliable, not user-deletable.
  - **Third-party (Claude Code, ClawHub/OpenClaw)** — the `ling-mem`
    skill + lifecycle hooks substitute for the engine scheduler
    (turn-counter on a Stop hook, etc.).
- Once Linggen ships built-in, the skill is **third-party-only**.
- The store lives under `~/.linggen`, owned by the binary, **not** in
  the skill bundle: deleting the skill degrades capture, never loses
  data.

### Recall

Live path is unchanged: core inlined every session; semantic queried via
`Memory_query` (verbs `get`/`search`/`list`) and surfaced at turn start.
Writes via `Memory_write` (`add`/`update`/`delete`). Recall spans
**both** tables: `Memory_query` queries semantic *and* episodic and
returns one merged, deduped result (semantic copy wins a near-dup tie).
Bulk forget stays user-initiated (dashboard / `ling-mem forget` CLI),
never a model tool.

### Write routing — by salience (step-3b)

Two write speeds, split by salience — mirrors the brain (explicit,
flagged input gets a fast strong trace; incidental experience goes
through slow hippocampal consolidation). Capture is **not**
episodic-only:

- **Explicit / declarative** — the user says "remember…", states an
  identity fact, gives a standing instruction, or uses commitment
  language ("always", "never", "from now on"). The live agent commits
  it **this turn** via `Memory_write` → semantic. Immediate, not queued
  behind the episodic→consolidate path. Salience → semantic *now*.
- **Incidental / observed** — what the user/agent did, a decision that
  emerged, ambient context. Goes to **episodic** via the every-N-turns
  encoder; the consolidator promotes past-TTL.
- **Core** is *not* a counter or a fast lane: salience gets a row to
  semantic; only a **stable universal** (name, role, enduring rule)
  becomes `tier=core`, user-confirmed or high-confidence identity-class.
  A volatile fact ("age 18") goes semantic but is a poor core
  candidate. The *kind* of fact decides core, never how many times it
  was said.

Redundancy across the two paths is safe and self-healing: cross-table
recall dedup keeps the semantic copy; the consolidator later deletes
the episodic twin.

**Repetition.** An *explicit* restatement reinforces via the live path
above. An *implicit* pattern (the user keeps doing A but never states a
rule) does **not** authorize the offline consolidator to synthesize
"user always A" — that generalization needs the user present (§3;
surfaced at recall, step 4). The consolidator may promote the
individual durable rows; it never mints a new general rule alone.

### Extraction contract

The encoder and the consolidator share one contract: the durability
rules in §4. Same rules, two runtimes.

## 3. Design rules

Six anchors. When a design decision is unclear, they break the tie.

**1. Human in the loop for destructive operations.**

Forgets, merges of distinct facts, and any rewrite that changes the
meaning of a row require explicit user confirmation. Append and
mechanical maintenance are the safe defaults. Never silently overwrite
a user-stated fact based on inference.

**2. The user is the source of truth.**

Memory records what the user said and what the agent observed in the
user's presence. The agent does not invent details — names, dates,
breeds, project terms — to make an entry feel complete. Fabricated
specifics mislead every future retrieval. If the user said *"a cat,"*
the row says *"a cat,"* not *"a cat named [made-up name]."*

**3. The file beats the memory.**

Anything re-derivable from workspace files (code, configs, project docs,
the project's own `AGENTS.md` / `CLAUDE.md`) doesn't belong in the memory
store. The file is the source of truth; memory storing the same content
creates a stale copy that rots on every refactor. Project-internal
architecture and conventions stay in those user-curated files. Memory
neither duplicates them nor writes back to them — drop the candidate.

**4. Curate, don't accumulate.**

The store grows with genuinely durable signal — the user's life, work,
and decisions accumulate over years, and that growth is the whole point.
What it must not do is drift: expired facts get retired, conflicting
rows get reconciled, noisy duplicates get merged. Net value goes up over
time, not row count alone.

**5. Consolidation promotes; only user-facing forgets need a human.**

The consolidator (§2) promotes episodic → semantic with agent judgment —
extract, generalize, supersede — autonomously, on the every-N-turns
tick. Episodic eviction is automatic (consolidate-before-evict). What
still requires explicit user confirmation: **bulk forget and any
destructive rewrite of an existing semantic row** (rule 1). Live
retrieval still reconciles/synthesizes in the user's presence. The old
"semantic maintenance = live only" split is retired.

The table below maps where each operation runs.

*Mechanical maintenance* — exact-rephrase dedup, extending contexts/tags,
adding `supersedes` links, retiring rows that meet a fixed obsolescence
rule — runs offline in the mission. The decisions are mechanical: a
rule fires, the action follows.

*Semantic maintenance* — merging facts into a story, generalizing
patterns into rules, choosing between contradicting rows — runs only in
the live session. The user is there to see the synthesis and correct it
before anything commits.

Bulk forget is user-initiated only, regardless of where it would run.

| Operation | Where | Why |
|:--|:--|:--|
| Append a new row | Offline mission OR live session | Pure additive |
| Exact-rephrase dedup (clearer wording on a near-duplicate) | Offline OR live | Mechanical — pick the better string |
| Extend `contexts[]` / `tags[]` from new evidence | Offline OR live | Mechanical — array union |
| Add a `supersedes` link between two rows | Offline OR live | Metadata only |
| Retire by fixed obsolescence rule (session-arc leak, hard TTL, completed supersedes chain) | Offline OR live | Mechanical — the criterion is a fixed rule |
| Merge distinct facts into a synthesized story | **Live only** | Content rewrite — hallucination risk |
| Generalize utterances into a "user always X" rule | **Live only** | Over-fit risk |
| Resolve a contradiction between rows | **Live only** | Needs context the offline run may lack |
| Bulk delete by filter | **User-initiated only — not a model tool** | The whole point is intentional cleanup; users invoke via the dashboard or `ling-mem forget` CLI. The model can iterate `Memory_query` → `Memory_write({verb: "delete"})` for small sets when explicitly asked. |

**6. Never store secrets, at any layer.**

Credentials, API keys, tokens, passwords, embedded auth in URLs — out
of memory entirely. The credential never enters any memory layer.
Memory does not write to project files, so there is no secondary
destination to consider. If the user wants to record a *gotcha* about
the credential (*"don't copy this URL into cloud configs"*), that's a
hand-edit they make to their own project file; memory does not author
those.

## 4. What's worth remembering

Memory's value is signal density, not row count. Three rules decide
whether a candidate earns its place. Scope (`tier=core` vs `semantic`)
is the §2 concern — these rules answer only the binary question:
**should this be saved at all?** Memory never writes to project files;
candidates that earn neither tier are dropped.

**1. Don't memorize what lives in workspace files.**

Code, configs, READMEs, project docs, the user's own `AGENTS.md` /
`CLAUDE.md` — the agent reads them when it needs them. Putting the
same content in memory creates a second copy that rots the moment the
file changes. The file is the source of truth; memory stays out of its
way.

> *"In repo1, the planner module exposes a facade that returns a
> context object per tick"* — **skip.** The agent will read the planner
> sources next time it matters. Memory does not auto-write to the
> project's `AGENTS.md` either — that file is user-curated. If the
> architectural intent is load-bearing for future work, the user can
> hand-edit it themselves.

This rule kills most "the codebase has X" candidates from offline scans.
If a fact can be re-derived by reading one or two files, it doesn't
belong in memory.

**2. User-stated preferences need a confidence gate.**

Not every *"the user said …"* line is durable. Distinguish three cases:

- **Save** — the user is correcting how the *agent* should work, with
  commitment language and cross-project reach:
  > *"I want the agent to always keep UI and server aligned, don't leave
  > one half-done into the next task."*

  This shapes agent behavior beyond a single repo. Record as
  `preference`.

- **Skip** — the user is making a single architectural call, true today
  and possibly reversed next month:
  > *"We should decouple layer 1 from the core engine."*

  Rot-prone. Belongs in design notes or the PR description — not memory.
  Memory does not auto-write to the project's `AGENTS.md` either; if
  the user wants it captured there, that's their hand-edit.

- **Record utterances; synthesize at retrieval, not at extraction.**
  When a pattern emerges across many sessions — repeated *"split this
  module"*, *"factor out Y"*, *"decouple X"* — the extractor still
  appends each one as its own row. It does **not** try to mint a
  higher-order preference like *"user prefers continual decoupling."*

  Synthesis happens live: when retrieval pulls several rows on the same
  theme, the agent reconciles them in prose — *"From memory: you've
  raised decoupling concerns in 5 sessions; pattern is X."* The user
  sees the generalization the moment it's made and can correct it.

  Why not synthesize offline: generalizing scattered utterances into a
  permanent rule is exactly where the agent over-fits — one strong rant
  can mint a "user always wants Y" claim that misrepresents them
  forever. Append-and-reconcile keeps the raw evidence and forces
  synthesis to happen in the user's presence.

  The proactive case — surfacing a pattern the user wouldn't have
  queried for — belongs in the dashboard, not the extractor. The
  dashboard can run cluster analysis on demand and offer *"we see N
  similar utterances about X — promote to a preference?"*, with the
  user confirming before any new typed row is written.

**3. User-only knowledge — record, then maintain.**

Facts only the user can supply: life context, history, relationships,
dates, equipment, the people and animals around them. The agent has no
other path to learn these, so when the user volunteers one, save it.
But every such fact ages, so:

- **Stamp ages relative to a date, not to "now".**
  > *"I have a 3-year-old cat"* → save as *"User has a cat, age 3 as of
  > 2026-04-27"*, not *"the cat is 3 years old."* Without the as-of
  > date, "3 years old" silently rots into "still 3 years old" forever.

  Record only what the user said. Don't invent a name, breed, or any
  other detail to make the entry feel complete — fabricated specifics
  will mislead every future retrieval.

- **Append at extraction; reconcile at retrieval.** When the user
  revises a fact, the offline extractor adds a new timestamped row — it
  does **not** overwrite the existing one. Reconciliation happens at
  read time: when multiple matching rows surface, the agent merges them
  in the response, ordered by timestamp, and the user sees the synthesis
  live (and can correct it on the spot).
  > Stored: *"User has a cat"* (2024). Later: *"When I relocated, I
  > left the cat with a friend"* (2026). Retrieval surfaces both;
  > the agent renders *"From memory: you had a cat that you left with a
  > friend during your 2026 relocation."*

  Why append rather than merge-at-write: a bad merge from the offline
  pipeline silently corrupts good data, and the user isn't there to
  catch it. Append-only keeps every original utterance recoverable; the
  agent's live synthesis is correctable in the same conversation. The
  raw timestamps also let the agent answer *"when did I get the cat?"* /
  *"how long did I have it?"* — questions a flattened row destroys.

  Optional hint: when the extractor is highly confident the new row
  supersedes an earlier one, it can tag the new row with a `supersedes:
  <id>` link. That's metadata for retrieval ranking, not a destructive
  edit.

  Destructive consolidation (actually deleting the old row) is
  user-initiated only — *"clean up my cat memory"* or a dashboard
  review. The agent proposes the merged version; the user approves
  before any write.

The extractor should pre-flag candidates that semantically overlap with
existing rows so the live agent (and the dashboard) sees the cluster at
retrieval time and can act on it with the user.

## Implementation pointers

This spec stays out of implementation. Where the wires actually run:

- **Tool dispatch and capability routing** — `tool-spec.md`,
  `skill-spec.md`.
- **Filesystem layout under `~/.linggen/`** — `storage-spec.md`.
- **Permission tiers and path scopes** — `permission-spec.md`.
- **Default RAG engine schema (locked shape)** —
  [linggen-memory/DESIGN.md](../../linggen-memory/DESIGN.md).
- **Session prompt assembly and the `include_memory` flag** —
  `session-spec.md`.

What Linggen assumes from any provider, regardless of implementation:
stable opaque row identity (Linggen never parses ids); free-form
many-to-many `contexts[]`; closed-enum `type`; provider-internal ranking
and embedding. Schema-versioned rows with explicit migrations. Daemons
bind localhost-only and are never exposed to remote consumers.

The default skill (`ling-mem`) ships two surfaces with separate
responsibilities: a **data UI** for row-level browsing (read-only on
open, every change explicit) and a **skill dashboard** for higher-level
summaries, extraction controls, and the on-demand cluster-analysis
described in §4 rule 2. The split is responsibility, not packaging.

## Evaluation

- **Retrieval** — [LongMemEval-S](https://arxiv.org/abs/2410.10813)
  (ICLR 2025), `GRANULARITY=turn` (ling-mem caps embeddings at 512
  tokens; session-level would measure the cap, not retrieval). Runner +
  methodology: `linggen-memory/benchmark/`. It measures the *retrieval
  subsystem only* — a regression check and a comparable number, **not**
  the system's worth. It cannot see extraction, consolidation, dedup,
  supersession, or decay, and it rewards the store-everything
  anti-pattern this design rejects. Frame any published number that way.
- **Write side** — extraction precision, dedup correctness,
  supersession accuracy, decay calibration. No standard benchmark
  exists; this is the eval that measures the hard part. Unbuilt — see
  Open / next.

## Open / next

Ordered. Each is a design decision not yet locked.

1. **Decay model** — *resolved* (§2): clock = `updated_at` (touch
   resets it); wall-clock `EPISODIC_TTL` (7d
   default, Settings-configurable); consolidate-before-evict; per-session
   every-N-turns trigger, no startup trigger, sub-N sessions skipped.
2. **Consolidator contract** — *resolved* (§2): consolidate processes
   past-TTL rows only; each is terminally promoted (→ semantic, optional
   `supersedes` link, never destructive) or deleted; re-entrancy needs
   no watermark — a handled row leaves episodic.
3. **Dedup threshold retune.** `DEDUP_SIMILARITY_THRESHOLD = 0.88` was
   tuned for v0.4 MiniLM; Qwen3-Embedding-0.6B's score distribution
   differs (relevant ~0.4–0.7). Re-validate before consolidation relies
   on it.
4. **Write-side eval — the next concrete deliverable.** Decided: do
   not optimize toward LongMemEval (it scores retrieval over a frozen
   store-everything haystack — structurally the opposite of curation;
   our designed system scores *lower* on it by construction, and that
   is correct). LongMemEval stays a regression check on the raw
   retrieval subsystem only (query pre-consolidation episodic,
   turn-granularity; add BM25+fusion to be respectable). The real
   scorecard is a new eval measuring extraction precision, dedup
   correctness, supersession accuracy, and decay calibration — unbuilt
   by anyone; this is the opening. Build it next.
5. **Encoder ↔ consolidator boundary** — *resolved* (§2): the encoder
   applies §4's hard exclusions **+ a write-time usefulness bar**
   (episodic is recall-visible immediately); the consolidator remains
   the terminal promote/delete gate for past-TTL rows. Revised
   2026-05-19 from "liberal capture" once recall began spanning episodic
   — server-side dedup (`insert_with_dedup`) already covers duplicates.
6. **Consolidation widget polish** — the tick currently reuses the
   generic subagent-tree surface. The §2 ideal (no-op ticks silent,
   persistent line only on a material change, a dedicated
   inspectable/undoable memory widget) is deferred.

Carried gaps (not blocking the rebuild): no row-level confidence
calibration; privacy isolation is by convention not enforcement;
cold-start has no importer; proactive pattern-surfacing lives only in
the dashboard.

## Future

- **Cross-device sync** — exports + git first; real sync is P2P via
  Linggen's WebRTC transport.
- **Temporal reasoning** — entity-time graph queries; `supersedes` is
  the structural foothold.
- **`Memory_archive`** — soft-forget: hidden from default search but
  recoverable.
