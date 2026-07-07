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

> **2026-05-28:** the every-N-turns memory encoder subagent was retired.
> Capture is now inline on the live agent's turn — same protocol across
> Linggen, Claude Code, Codex, and OpenClaw. The `dream` mission still
> owns bulk consolidation. §2 below reflects the new architecture.

> **2026-07-03:** dream redesigned day-granular with three stages —
> **harvest, remember, forget**. Remembering (consolidation) no longer
> deletes; episodic is short-term memory kept for its full TTL. Per-day
> dream state lives in ling-mem as the single truth for the mission,
> the calendar UI, and third-party hosts. §2 reflects the new pipeline.
> *(Stage naming superseded 2026-07-06: harvest → **scan**, pulled out
> of dream; **condense** added as stage 4 — see the notes below.)*

> **2026-07-06:** the offline half is now **two user-facing functions**:
> **scan** (per-day backfill staging, incremental by `source_session`)
> and **dream** (= remember + forget). On Linggen every dream trigger
> converges on the day-parameterizable `dream` mission — the skill owns
> only scan and the calendar UI, never a second dream implementation.

> **2026-07-06 (merge law):** merge doctrine rewritten around **voice**:
> the agent merges and rewrites its own notes (`from=derived`) freely,
> always via `replace_ids`; the user's voice (`from=user`) changes only
> with the user. Semantic holds **state + lessons, never events**.
> **Condense** joins as stage 4 — the only pass over semantic-at-rest.
> §2 Reconcile, §2 Condense, and §3 rules 1/5 carry the new law.

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
- **Episodic** — short-term memory: recent experience, kept for its
  full TTL even after the dream pass has judged it (like human short-
  term memory, it lingers, then fades). **On the recall path** (recall
  spans both tables — see §2 Recall) *and* the dream pipeline's input.

If a candidate earns none of the three, **drop it.** Memory never writes
to project files (`AGENTS.md`, `CLAUDE.md`, source, docs) — those are
user-curated; the agent reads them directly when needed.

## 2. Architecture — capture, consolidate, recall

A two-engine pipeline over three stores. The model is the hippocampus:
fast broad capture during waking, selective consolidation offline,
forgetting of what didn't earn permanence.

### Stores — two LanceDB tables, no markdown

- **Semantic table** — durable, curated, append-mostly (changed by a
  new row + read-time reconciliation, `replace_ids` merges of the
  agent's own notes, or explicit user edit/delete — user-voice rows are
  never silently rewritten; §3 rules 1/5). Rows
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

Two paths, split by the Complementary Learning Systems model — fast
encoding during the turn, slow consolidation + forgetting "asleep":

**Capture — inline, on every turn.** The live agent currently talking to
the user (`ling` in Linggen; the host-resident agent in Claude Code,
Codex, OpenClaw) writes durable rows in the same turn the signal arrives.
It is driven by:

- the system prompt's memory protocol — the substantive doctrine of
  *what to save and when*, shipped as the MCP server's `initialize.instructions`
  (third-party hosts) or inlined by `engine/prompt` (Linggen);
- the per-turn auto-recall hint — relevant prior rows surface at turn
  start so the live agent can chip them in its reply *and* reconcile
  duplicates/conflicts on the side (see Reconcile).

Each `Memory_write` reads existing memory first (`Memory_query`): skip a
duplicate (exact or reworded); on a contradiction call the host's
ask-user primitive (Linggen `AskUser`, Claude Code `AskUserQuestion`,
plain chat on Codex/OpenClaw) and write the resolved row on answer. Same
contract across every host — the *kind* of fact at stake (preferences,
identity, declarative statements) is the same kind the live agent
already produces, so a second model running the same prompt out-of-band
adds latency without lifting capture quality. ≈ waking encoding.

This was previously an every-N-turns `ling-mem` subagent on Linggen
(retired 2026-05-28). Cross-host parity, no AskUser bridge layer, no
per-session counter, no SubagentPane routing.

Cross-tool memory comes from the **shared store** — one `~/.linggen`
backed by the daemon, every host writes through `Memory_*` — never from
one tool scraping another's logs.

**Two user-facing functions** drive the offline half: **scan** (stage
what live capture missed) and **dream** (turn staging into long-term
memory, then age the rest out). Dream = remember + forget; scan is a
separate, always user-triggered action — never part of the unattended
run.

**Scan — per-day backfill staging.** For a past day the user picks,
walk host session logs (the `shared-memory` skill's scan scripts —
zero-LLM; the binary stays out of session files) and encode that day's
worthwhile candidates into episodic carrying the day's own timestamps,
stamping the day scanned. **Incremental by `source_session`**: sessions
that already contributed rows (live capture or a prior scan) are
skipped, making scanning idempotent — safe even on partially-captured
days. (Contract, not yet reality: live capture doesn't stamp
`source_session` today, so the skip only covers scan-authored rows —
Open #9.) Scan matters exactly where live
capture wasn't running (host without the memory hook, daemon off,
pre-install history); a day nobody worked scans to nothing, which is
why it stays a per-day human call. A gap day simply sits on the
calendar until backfilled.

**Dream — the built-in `dream` mission, the single executor.** A
normal, **visible** built-in mission (`~/.linggen/missions/dream/`,
installed once on setup/upgrade, user-stoppable per run, deletable as
a supported opt-out). Dream is **day-granular**: it walks **pending
days, oldest first** (a day is pending when it has episodic rows not
yet judged) and runs two strictly-ordered phases per pass:

1. **Remember** — consolidation. Judge one pending day's episodic rows:
   skip what long-term already holds, pick one representative per
   near-duplicate cluster, promote durable signal → semantic (an
   append, or a `replace_ids` merge into the agent's own derived rows —
   user-voice rows are never rewritten; §3 rule 5), then stamp the day
   `remembered_at` in the per-day dream state. **Remembering never
   deletes** — episodic rows stay as short-term memory for their full
   TTL. (One mechanical exception: a *verbatim* promote trips the
   store's cross-tier exact-dup collapse, which removes the
   byte-identical episodic twin — the content lives on in semantic, so
   nothing is lost.) Generalizing utterances into rules and resolving
   user-voice contradictions stay user-present (Reconcile, below). Any pending day qualifies,
   including yesterday — consolidation latency is one night, decoupled
   from TTL. May propose **semantic → `tier=core`** for a stable
   universal (user-confirmed; rule 1).
2. **Forget** — mechanical eviction, **no LLM**: delete episodic rows
   past `EPISODIC_TTL` whose day is remembered and whose creation
   precedes the day's `remembered_at`. Nothing is deleted unjudged: an
   undreamed day's rows survive past TTL until a remember pass judges
   them. Rows arriving after `remembered_at` (late capture, scan
   backfill, cross-host lag) re-pend the day for a follow-up remember.

The phase contracts compose into the full decision table with no
per-case logic: a recent pending day gets remembered and keeps its
episodic (not yet past TTL); an old remembered day is forgotten
mechanically; an old never-dreamed day gets the full pass — remember,
then forget. Remember writes only promotions (no per-row deletes), so
a dream run's cost is bounded by one day's durable signal, not its row
count.

**Per-day dream state lives in ling-mem** — `{date, harvested_at,
remembered_at, per-run counts}` (`harvested_at` is the scan stamp; the
field keeps its original name), a small sidecar beside the two tables
(not memory rows; the frozen store schema is untouched). It is the
single source of truth read by the mission, the memory app's calendar,
and third-party hosts, exposed as a per-day rollup (`days`: today /
staging / pending / remembered / forgotten / harvested) over the CLI
and HTTP. State is
stored, not derived, because remembering keeps rows — "judged" can no
longer be inferred from row absence.

`EPISODIC_TTL` default **7 days** (Settings) — purely the short-term
retention length, no longer a consolidation gate. Losslessness comes
from **forget-only-touches-remembered-days**, replacing the old
consolidate-before-evict ordering. Episodic is therefore bounded and
lossless except for rows the remember pass judged not worth promoting —
which is correct.

**Schedule = daily cron + turn-seam catch-up.** Cron alone is missed
when the machine is off/asleep, so on each completed owner turn, if the
mission has not run within `dream_catchup_hours` (default 24) it is
triggered as a catch-up — the next time Linggen is used after a missed
night. A shared in-flight guard prevents a cron run and a catch-up (or
two catch-ups) from overlapping, and catch-up retries are **capped per
day** so a failing run cannot re-fire indefinitely. A run that sees the
same worklist twice in a row aborts as stalled rather than looping.
Each run produces a mission run-record (the audit trail of automated
promotes) and is per-run stoppable; no-op runs stay quiet. A completed
run ends with an **engine-composed report** appended to the run session
— worklist, per-day remembered counts, sweep evictions — built
mechanically from the tool results, never from model prose (the model's
own status lines are best-effort narration only).

**One dream executor per host.** On Linggen, every dream trigger —
nightly cron, turn-seam catch-up, the memory app's Run-dream button, a
calendar day's dream button — converges on a `dream` **mission run**
(day-scoped via the trigger's `day` argument when a specific day was
clicked): same `memory` agent, same in-flight guard, same run record
and engine-composed report on every path. The skill never
re-implements the dream procedure; it owns **scan** (its session
scripts need the skill's tools) and the calendar UI. Without Linggen
there is no mission runtime — on Claude Code / Codex / OpenClaw the
`shared-memory` skill's runbook drives the same daemon primitives
(fetch pending days, remember the earliest, invoke forget) from the
host agent, on demand or via the host's own scheduler.

The memory app's **calendar** renders the per-day `days` rollup as two
controls per past day, each explained by a tooltip: `scan | scanned`
and `dream (n) | dreamed` (today is disabled — a day is judged only
once it's over). State lives in the daemon, so a day reads `dreamed`
identically whether the nightly run or a day button got there.

### Binary vs judgment split

- The **`ling-mem` binary** owns both tables, the per-day dream state,
  and the mechanical primitives (embed, dedup, the `days` rollup, the
  `forget` sweep). **No LLM in the binary** — it is the portable,
  deterministic data layer, the single shared source every caller goes
  through.
- **Judgment** ("is this worth saving", "is this a contradiction") needs
  an LLM. Capture judgment lives in whichever agent is currently talking
  to the user — `ling` on Linggen, the host-resident agent on
  Claude Code / Codex / OpenClaw. Offline judgment (the remember stage)
  lives in the built-in **`memory` agent** (`agents/memory.md`,
  tools = `Memory_query` + `Memory_write` only, unattended-safe: no
  AskUser, uncertainty resolves to promote). Every entry point borrows
  that one brain: the `dream` mission runs under it (`agent: memory`),
  the memory app's day buttons trigger day-scoped runs of that same
  mission, and on third-party hosts the `shared-memory` skill's dream
  runbook drives the host agent through the same daemon primitives
  (`days`, day-scoped `list`, `remember_day`, `sweep` — CLI or MCP).
  One procedure, N triggers.
- The store lives under `~/.linggen`, owned by the binary, **not** in
  the skill bundle: deleting the skill / plugin degrades capture, never
  loses data.

### Recall

Live path is unchanged: core inlined every session; semantic queried via
`Memory_query` (verbs `get`/`search`/`list`, plus `days` for the
per-day dream-state rollup) and surfaced at turn start. Writes via
`Memory_write` (`add`/`update`/`delete`, plus the dream verbs
`remember_day`/`sweep`). Recall spans
**both** tables: `Memory_query` queries semantic *and* episodic and
returns one union — **exact-content** duplicates collapsed (semantic
copy wins); a high-cosine *contradiction* is deliberately kept so recall
surfaces both for the user/LLM to reconcile, never silently hidden.
Bulk forget stays user-initiated (dashboard / `ling-mem forget` CLI),
never a model tool.

### Write routing — by salience

Two write speeds, split by salience — mirrors the brain (explicit,
flagged input gets a fast strong trace; incidental experience
accumulates and is later consolidated). Both happen inline on the
same turn — the difference is the target table, not who writes:

- **Explicit / declarative** — the user says "remember…", states an
  identity fact, gives a standing instruction, or uses commitment
  language ("always", "never", "from now on"). The live agent commits
  it **this turn** via `Memory_write` → semantic (`tier=core` for
  stable universals; `tier=semantic` otherwise). Immediate.
- **Incidental / observed** — a decision that emerged, ambient context
  the agent judged worth preserving across sessions. The live agent
  writes it to **episodic** in the same turn. The dream pass remembers
  the durable rows the following night; forget evicts the rest after
  `EPISODIC_TTL`.
- **Core** is *not* a counter or a fast lane: salience gets a row to
  semantic; only a **stable universal** (name, role, enduring rule)
  becomes `tier=core`. A volatile fact ("age 18") goes semantic but is
  a poor core candidate. The *kind* of fact decides core, never how
  many times it was said.

**Semantic holds state + lessons, never events.** The routing test:
strip the date and the commit hash — still useful in three months?
Per-event work rows ("committed X", "pushed Y", "closed the session")
go to **episodic always**, however salient the turn felt; the dream
pass folds them into whatever durable state they evidence. State
changes ("X is now shipped") and re-hit gotchas are the semantic
material. Episodic's TTL is the event horizon — if 7 days proves
short, the Settings knob bumps it (14), not the routing.

Redundancy across paths is safe and self-healing: a byte-identical
episodic twin is collapsed into the semantic copy at promote time
(cross-tier write dedup); a reworded twin is kept by recall dedup
(semantic copy wins on exact match) and ages out at the forget stage.

**Repetition.** An *explicit* restatement reinforces via the live path
above. An *implicit* pattern (the user keeps doing A but never states a
rule) does **not** authorize the `dream` mission to synthesize
"user always A" — that generalization needs the user present (§3;
surfaced at recall, step 4). Dream may promote the individual durable
rows; it never mints a new general rule alone.

### Reconcile

Memory is reconciled against existing memory as an **ambient
responsibility, on any reactivation** — not at fixed seams. Core is
re-injected every turn, recall surfaces rows at turn start and mid-task,
write compares a candidate, the dream run holds a worklist: each is a
reconcile opportunity. (Brain-faithful: reconsolidation fires on
reactivation, and core is reactivated every turn.)

**The merge law — authority follows voice.** Every row is either the
agent's note or the user's word, and that decides who may change it:

- **The agent's notebook** (`from=derived` — `built`, `fixed`, `tried`,
  `learned`): the agent merges, rewrites, and retires its own notes
  freely, at any seam — write, recall, dream, condense — always via
  `replace_ids`, so one call inserts the survivor and deletes every
  listed loser (a concurrent recall sees the old rows or both, never an
  empty hole on the subject; surfaces without `replace_ids` order add
  then delete).
- **The user's voice** (`from=user` — preferences, decisions, identity):
  changes only with the user. Surface the conflicting rows **with their
  dates** and ask via the host's ask-user primitive (Linggen `AskUser`,
  Claude Code `AskUserQuestion`, plain chat on Codex/OpenClaw); write
  the resolved row with `replace_ids` carrying the losers. When the
  conflict is **not material** to the turn, or the context **cannot
  ask**: append the new row, leave the old — recall keeps surfacing
  both until a user-present seam resolves it.

**See-it-solve-it.** Whichever agent surfaces garbage owns it in that
moment — there is no cleanup queue. The taxonomy:

| Seen | Action |
|:--|:--|
| Exact duplicate (byte-identical, same type) | Mechanical collapse — the binary does this on write |
| Superseded / chain member, derived ("impl not started" → "shipped") | `replace_ids` merge into one current-truth row |
| Reworded derived near-dup | Merge, keep the best phrasing |
| Old pure-event row | Mechanical retire — fold into a state row if one exists |
| User-voice contradiction | AskUser with dates; can't ask / not material → append, leave both |
| Secret | Delete |
| New | Write |

Cosine **cannot** separate a contradiction from a restatement — both
score high — so everything below the first row needs the LLM, never
the binary's dedup. The tact rule stands: this is for *incidental*
hits — when the user is explicitly steering memory, follow their
instruction; don't side-quest into cleanup.

**Same protocol across every host.** Two write surfaces remain:

- **Live `Memory_write`** (the live agent, on every turn) — reads first;
  applies the taxonomy; user-voice conflicts go through the host's
  ask-user primitive.
- **Unattended missions** (`dream` nightly, `condense` monthly — no
  user present) — promote durable signal and freely merge the agent's
  own derived rows via `replace_ids`; user-voice rows are never
  rewritten there — those conflicts wait for a user-present seam.

The encoder subagent and its dedicated `SubagentPane` widget routing are
**gone** — the live agent's ask-user widget lands wherever it is
already talking (Linggen's main chat, CC's UI, Codex's terminal). Same
contract on all four hosts.

### Condense — stage 4, semantic-at-rest maintenance

Semantic is append-mostly, so project truths accumulate **chains** —
same-subject rows where the newest completes or obsoletes the rest
("design locked, impl not started" → "shipped"). Every other merge
point gates entry (write-time dedup, the dream's promotion judgment)
or works a recall window (turn-start recall, read-before-write);
**nothing revisits semantic-at-rest**. Condense is that pass — the
only one whose input is old semantic rows.

The cure is **merge, not link**: a chain is *computed* at detection
time — never a stored edge (the removed `supersedes` link stays
removed; a link leaves the stale row recallable and adds graph
traversal to every read). Condensing replaces N rows with one
current-truth row whose content carries the history as narrative with
its dated span, applied via `replace_ids` under the merge law: derived
chains collapse freely; a chain touching user-voice rows is never
auto-merged (supervised runs may ask; unattended runs skip it).

- **v1 — chain-collapse.** Two confidence tiers: a *tier-1* edge is a
  verbatim row-ID citation inside another row's content — mechanical,
  auto-accepted; a *tier-2* candidate carries only provisional-state
  markers ("uncommitted", "OPEN:", "pending") — found by marker +
  search, LLM-confirmed before merging.
- **v2 — subject digests.** Focused per-subject current-truth rows —
  never one mega state row.
- **Runs as a monthly mission** under the same `memory` agent. Ships
  cron-disabled; first runs supervised, with a `ling-mem export`
  backup taken before the pass.
- **Split.** The mechanical scan is a new **`chains` verb in the
  ling-mem daemon** — the mission agent is Memory-tools-only and can't
  script the scan, and doing it in-context costs ~140k tokens.
  Judgment (confirm tier-2, draft the merged row) lives in the linggen
  mission + `memory` agent; the procedure doc lives in the
  `shared-memory` skill.

**Floors.** Synthesis *for the answer* is always encouraged (merge rows
into a dated narrative in the reply). Offline synthesis *into a stored
row* is forbidden exactly where the false-memory risk lives —
user-voice rows (rule 1); the agent's own notes carry no such floor
(rule 5). Bulk forget stays user-initiated.

### Extraction contract

The live agent (capture) and the `dream` mission (consolidation) share
one contract: the durability rules in §4, plus Reconcile above. Same
rules, runtimes differ only by user-presence.

## 3. Design rules

Six anchors. When a design decision is unclear, they break the tie.

**1. Human in the loop for destructive operations on the user's voice.**

Bulk forgets, and any merge or rewrite that changes the meaning of a
row **in the user's voice** (`from=user` — preferences, decisions,
identity), require explicit user confirmation. Append and mechanical
maintenance are the safe defaults. Never silently overwrite a
user-stated fact based on inference. The agent's own notes are
governed by rule 5.

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
time, not row count alone. **Ownership is see-it-solve-it** (§2
Reconcile): whichever agent surfaces the garbage owns fixing it in that
moment; what no recall ever touches falls to the condense pass (§2).
Curation has no other queue.

**5. Merge authority follows voice — the notebook/voice law.**

The agent's own notes (`from=derived`: `built`, `fixed`, `tried`,
`learned`) are its notebook: it merges, rewrites, and retires them
freely — in dream, at recall, in condense — always via `replace_ids`
so the survivor lands atomically. Rows in the user's voice
(`from=user`: preferences, decisions, identity) change only with the
user present (rule 1): AskUser first; otherwise append and reconcile
at read time.

Why the split: append-only defers the merge cost onto every future
recall — the moment with the *least* context — while the false-memory
risk that motivated "never silently merge" concentrates almost
entirely in user-voice rows, not the agent's notes. (Claude Code's own
auto-memory, the alignment reference, trusts the model to rewrite its
notes.)

Two judgments stay user-present regardless of voice: generalizing
utterances into a "user always X" rule, and resolving user-voice
contradictions (§2 Reconcile). Bulk delete by filter is user-initiated
only — dashboard or `ling-mem forget` CLI, never a model tool; the
model can iterate `Memory_query` → `Memory_write({verb: "delete"})`
for small sets when explicitly asked.

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

  There is no structural "replaces" link: a newer row does not point at
  the row it obsoletes. Currency is inferred at read time from
  timestamps + content (a future link primitive is noted under Future,
  not built). The older row simply remains until explicitly forgotten.

  Destructive consolidation (actually deleting the old row) is
  user-initiated only — *"clean up my cat memory"* or a dashboard
  review. The agent proposes the merged version; the user approves
  before any write.

This whole rule is the **Reconcile** contract (§2) applied to user-only
facts: append timestamped rows; reconcile when the user is present;
never merge-at-write offline. Note that *similarity* surfaces the
cluster but cannot tell a restatement from a contradiction ("cat is
male" vs "cat is female" score nearly identical) — separating those is
the user-present LLM judgment step, not the binary's dedup.

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
  reconciliation, or decay, and it rewards the store-everything
  anti-pattern this design rejects. Frame any published number that way.
- **Write side** — extraction precision, dedup correctness,
  reconciliation accuracy, decay calibration. No standard benchmark
  exists; this is the eval that measures the hard part. Unbuilt — see
  Open / next.

## Open / next

Ordered. Each is a design decision not yet locked.

1. **Decay model** — *resolved*, revised 2026-07-03 (§2): clock =
   `updated_at` (touch resets it); wall-clock `EPISODIC_TTL` (7d
   default, Settings-configurable) is **pure short-term retention**, no
   longer a consolidation gate. Forget is a mechanical sweep gated on
   remembered days (past TTL + day remembered + row created before
   `remembered_at`). Capture is inline on every live turn (no encoder
   subagent — retired 2026-05-28); the dream pipeline runs on the
   `dream` mission (daily cron + capped turn-seam catch-up).
2. **Consolidator contract** — *resolved*, revised 2026-07-03 (§2):
   **day-granular** — walk pending days oldest-first; remember = judge
   one day's rows, promote durable signal (plain append, never
   destructive), stamp `remembered_at`; **never deletes**. Re-entrancy
   comes from the per-day dream state in ling-mem (late-arriving rows
   re-pend the day), replacing "a handled row leaves episodic". The
   `dream` mission body is maintained in the linggen repo (installed
   to `~/.linggen/missions/dream/`); the skill's dream runbook
   documents the same procedure for third-party hosts — aligned by
   hand, neither generated from the other. Prior model
   (past-TTL worklist, terminal promote-or-delete per row) retired
   2026-07-03 — it coupled consolidation latency to TTL and spent most
   of its tokens on per-row deletes.
3. **Dedup threshold** — *resolved, then retired as a gate*: mechanical
   dedup is exact-content only (cosine is never a sameness gate).
   `DEDUP_SIMILARITY_THRESHOLD = 0.75` (retuned 2026-05-15 against the
   real Qwen3-Embedding-0.6B distribution: restatements 0.78–0.97,
   distinct pairs ≤0.65) survives in `linggen-memory` only as a
   relevance/tuning reference, never a merge gate.
4. **Write-side eval — the next concrete deliverable.** Decided: do
   not optimize toward LongMemEval (it scores retrieval over a frozen
   store-everything haystack — structurally the opposite of curation;
   our designed system scores *lower* on it by construction, and that
   is correct). LongMemEval stays a regression check on the raw
   retrieval subsystem only (query pre-consolidation episodic,
   turn-granularity; hybrid scoring — cosine + IDF-weighted keyword
   boost — shipped 2026-06-09). The real
   scorecard is a new eval measuring extraction precision, dedup
   correctness, reconciliation accuracy, and decay calibration — unbuilt
   by anyone; this is the opening. Build it next.
5. **Capture ↔ consolidation boundary** — *resolved* (§2): the live
   agent applies §4's hard exclusions **+ a write-time usefulness bar**
   (episodic is recall-visible immediately); the dream pass remains
   the promotion gate (remember), with eviction a separate mechanical
   sweep (forget). Revised
   2026-05-19 from "liberal capture" once recall began spanning episodic
   — server-side dedup (`insert_with_dedup`) collapses byte-identical
   restatements (exact-content only; reworded/contradiction is the LLM's).
   Updated 2026-05-28: capture moved off a per-session subagent onto the
   live agent's inline turn writes.
6. **Reconcile contract** — *resolved*, revised 2026-07-06 (§2
   Reconcile): ambient trigger (any reactivation), see-it-solve-it
   ownership, merge authority by voice — derived rows merge freely via
   atomic `replace_ids`; user-voice contradiction→AskUser(dated),
   no-user→append and leave both. Exact-dup→mechanical collapse;
   cosine is never a sameness gate (it can't tell a restatement from a
   contradiction — that classification needs the LLM). Offline
   synthesize-into-storage / destructive-delete forbidden for
   user-voice rows only (was: everywhere); no structural replaces-link
   (append + read-time + user delete). One contract, every call site,
   differ only by user-presence. Implementation mechanism (shared
   engine module vs shared prompt) deferred to build time.
7. **Consolidation widget polish** — the dream mission now carries a
   per-run mission record (the promote/delete audit trail) and is
   per-run stoppable; no-op runs stay quiet. Still deferred: a
   dedicated inspectable/undoable *memory* widget distinct from the
   generic mission/subagent surface.
8. **Condense — designed 2026-07-06, unbuilt.** (Second design that
   day; supersedes the same-day condense-*queue* draft — see-it-solve-it
   replaced ambient enqueueing.) §2 Condense: v1 chain-collapse (tier-1
   ID-citation edges auto-accept, tier-2 marker+search LLM-confirm),
   v2 subject digests; monthly mission, ships cron-disabled, supervised
   first runs with `ling-mem export` backup; new `chains` verb in the
   ling-mem daemon for the mechanical scan. Store audit behind it
   (2026-07-06, 748 semantic rows): 151 provable tier-1 chain members,
   277 asserting provisional state ("uncommitted"/"OPEN:"), 71% written
   directly by hosts bypassing episodic staging — the state+lessons
   routing (§2) closes the faucet; condense drains the pool. Also from
   the audit: `tier=core` is empty (never seeded post-wipe) — seeding
   still pending.
9. **`source_session` gap — open.** No episodic row today carries
   `source_session`: the write schema documents that the engine fills
   it, but no engine code does, and third-party hosts never pass it.
   Scan's skip-by-session idempotency (§2) is therefore contract, not
   reality — it currently holds only for scan-authored rows. Fix:
   the engine stamps `source_session` on every live write; hosts pass
   their session id where they have one.

Carried gaps (not blocking the rebuild): no row-level confidence
calibration; privacy isolation is by convention not enforcement;
cold-start has no importer; proactive pattern-surfacing lives only in
the dashboard.

## Future

- **Cross-device sync** — exports + git first; real sync is P2P via
  Linggen's WebRTC transport.
- **Structural revision link + temporal reasoning** — a `replaces`
  link (newer row → the row it obsoletes) plus entity-time graph
  queries. Deliberately *not* built: it is a schema change (→ a
  store wipe under the no-forward-migration policy) and reconciliation
  works without it (append + read-time + explicit user delete). Revisit
  only if read-time reconciliation proves insufficient at scale.
- **`Memory_archive`** — soft-forget: hidden from default search but
  recoverable.
