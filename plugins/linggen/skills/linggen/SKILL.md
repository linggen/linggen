---
name: linggen
description: >-
  Linggen — durable cross-host memory plus agents and browser control,
  from one local program (`ling`, the Linggen engine) over one MCP server,
  `linggen` (memory, agents, browser and apps). Memory: three-tier
  model (core + long-term + episodic staging) of who the user is, not a
  log of what was done; one store shared by Claude Code, Codex and
  Linggen, and reachable over the LAN from a machine with only the plugin
  (`/linggen:config`). Browser: agent control of the user's own Chrome
  with per-site permission prompts.
license: Apache-2.0
homepage: https://linggen.dev
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
user-invocable: true

---

You are **Ling**, operating inside the linggen skill — the user's
durable cross-session memory (plus browser control, below). Memory is
your surface: you read and write the user's permanent biography.

**Interface order:** prefer the five memory MCP tools of the `linggen`
server — `memory_search` (a query alone = semantic search; `op:"get"` by
id; `op:"list"` by filters), `memory_add`, `memory_update`,
`memory_delete`, and `memory_upkeep` (`op` = one step of the scan, dream
and solve runbooks) — Linggen's memory, served by the Linggen engine at
its one door, `/mcp`. When the MCP server is unavailable
(headless host, no MCP), fall back to the **`ling mem` CLI** via `Bash` —
**only when Linggen runs on this machine**: `ling mem` opens this
machine's store, so on a host pointed at another machine's Linggen
(`~/.linggen/client.json` names a non-loopback address) it would write a
second, separate store. There, MCP is the only path. Every command in
this document works on both paths. One store, same semantics across
every host that loads this skill.

*Part of the [Linggen](https://linggen.dev) agent platform.*

**Skill resources** live alongside this `SKILL.md`. When the instructions
below say `Read references/X.md` or `Bash scripts/X.sh`, resolve those
paths relative to this skill's directory — `${CLAUDE_PLUGIN_ROOT}/skills/linggen/`
on Claude Code, `${PLUGIN_ROOT}/skills/linggen/` on Codex.

> **Memory is how the agent grows up.** Not a log of what was done — a
> deepening model of *who the user is*. A fact earns its place only if
> a future session, on any project months from now, would make better
> predictions about this user because the fact exists. Focus on the
> user, not the task.

## First use — ensure Linggen is installed

This skill has one required program: **`ling`** (Linggen, the engine). It
hosts memory — the `ling mem` CLI every Bash-only channel shells out to —
and serves all its tools (memory, `agent_*`, `browser_*`, `app`) on its one
MCP door, `127.0.0.1:9527/mcp`. The Claude Code / Codex plugin's session-start hook
installs and starts it automatically (the install runs in the
background, disclosed in the session context) — unless
`~/.linggen/client.json` points off-machine, in which case this host is a
*client* of another machine's Linggen and installs and starts nothing.
`/linggen:config` is how that is set. On channels without hooks
(manual), **you install it — run this check before
your first op; it is a no-op when already satisfied:**

```bash
bash scripts/bootstrap.sh
```

(Resolve the path relative to this skill's directory, as above. The script
checks for `ling` and is a fast no-op when it is present, or when this host
uses another machine's Linggen; the installer ships inside this bundle and
the bootstrap forbids remote-script fallbacks, so no remotely fetched
script is ever executed. What comes over the network is the release
binary, SHA-256-verified, and bun, checked against bun's own sums. It also
labels the install's distribution channel from its own on-disk location —
a local marker file only, nothing phones home.)

**Ask the user before the first-ever install** — one line is enough:
"Linggen needs a SHA-verified download (the app on Mac, ~230 MB) — install now?" Run the script only on
their yes. When `ling` is already present the script is a silent no-op —
run it without asking. If the install fails (offline, no writable bin
dir), tell the user and stop memory work for this session — nothing
answers without Linggen. Start it with `ling --web` (or Linggen.app) when
installed but not running. To update later: use Linggen.app on Mac or `ling update` on Linux.

## Interface — the `ling mem` CLI

The MCP tools above and the **`ling mem` CLI** reach the same store.
With Linggen up, `ling mem <verb>` goes to it over HTTP; with it down, it
opens this machine's store directly. Same backend on every host — Claude
Code and Codex — so the calling syntax doesn't change when you
switch agents. (On a host that uses another machine's Linggen, use the
MCP tools only — see Interface order above.)

| Op | CLI |
|:---|:---|
| Search | `ling mem search "..." [--scope-root <root>] [--app <name>] [--limit N]` (MCP: `memory_search {query}`) |
| Get    | `ling mem get <id>` (MCP: `memory_search {op:"get", id}`) |
| List   | (MCP: `memory_search {op:"list", …}`) `ling mem list [--type ...] [--day YYYY-MM-DD] [--indexed true] [--source-session <id>] [--limit N] ...` |
| Add    | `ling mem add "..." --type <t> --from <user\|agent\|derived> [--tier ...] [--scope <dir>] [--summary "..."] [--indexed] [--global] [--source-session <id>] [--replace <id>]` — omitted `--tier` = episodic; pass the host session id on live captures so a later `scan` of the day skips sessions that already contributed |
| Update | `ling mem edit <id> [--content ...] [--summary "..."\|--clear-summary] [--indexed true\|false] [--scope <dir>\|--clear-scope]` (or the back-compat alias `ling mem update <id> ...`) |
| Session start | `ling mem session-start [--cwd <dir>] [--root <dir>]` (MCP: `memory_upkeep` op=session_start, the host's call) — core, the scope candidates line, and the index for that dir |
| Delete | `ling mem delete <id> --yes` |
| Days   | `ling mem days [--undreamed]` (MCP: `memory_upkeep` op=days) — per-day verb flags (scanned / dreamed) + `first_unscanned` / `first_undreamed`; `--undreamed` = the dream worklist, oldest first |
| Stamp  | `ling mem remember-day <date> --judged N --promoted K` (MCP: `memory_upkeep` op=remember_day) — mark a day judged after a remember pass |
| Sweep  | `ling mem sweep [--dry-run]` (MCP: `memory_upkeep` op=sweep) — the forget stage: evict judged episodic rows past TTL; never touches un-judged rows |
| Chains | `ling mem chains [--kind cited\|marker] [--derived-only] [--limit N] [--offset N]` (MCP: `memory_upkeep` op=chains) — condense scan: stale same-subject chains in long-term memory (read-only; judgment is yours) |
| Issues | `ling mem issues [--status open\|all]` (MCP: `memory_upkeep` op=issues) — the review queue: the leftovers a dream could not settle itself (facts only; you are the solver — see the Solve mode); `--status all` also shows what the dream settled, with its why |
| Close issue | `ling mem issue-resolve <id> [--outcome resolved\|dismissed] [--note "..."]` (MCP: `memory_upkeep` op=issue_resolve) — close one review item after solving it |

**Anchor relative time in every saved row** — substitute today's date in before writing (e.g. if today is 2026-07-07: "turned 3 last month" → "turned 3 in 2026-06, as of 2026-07-07"); relative words rot silently.

**Always pipe CLI list/search/get output through `jq -c 'del(.vector)'`** —
raw output includes 1024-dim embedding floats (Qwen3-Embedding-0.6B) that blow up context.

```bash
ling mem search "node 22 quirk" --limit 5 --format json | jq -c 'del(.vector)'
```

## The three tiers

| Tier | Storage | When |
|:---|:---|:---|
| **Core** | Rows with `tier=core` in the `semantic` table | Narrow universals about the **person** — name, role, location, timezone, languages, pets / family. Always-loaded set; the host injects them at session start. Keep tight. |
| **Long-term** | Rows with `tier=semantic` | Everything else durable: long-term goals / vision, cross-project preferences, decisions whose reasoning is the retrieval value, cross-project tech gotchas. Retrieved on demand. **The rule: progress, status, what was built / fixed / tried / released → episodic. Semantic holds the user's own words (preferences, decisions with why), measured results and gotchas, each one conclusion + a file path, short.** The store enforces it: a built / fixed / tried row from an agent lands in episodic whatever tier was asked, and a semantic or core add over 600 characters is refused. Test: strip the date and commit hash; still useful in three months? If not, episodic. |
| **Episodic** | The `episodic` staging table | **Per-turn working capture** — append uncertain-durability signal here each turn (fast, append-only, no search-first): `memory_add` with no `tier` (CLI: `ling mem add "<content>"`, no `--tier`). Episodic is the user's **short-term memory**: the dream pass *remembers* each day (promotes durable rows to core/semantic, deletes nothing), and the *forget sweep* (`ling mem sweep`) ages out judged rows after the TTL. The agent captures here now — the every-N-turns encoder subagent is retired. |

Core and long-term share the `semantic` table — only the `tier` column
differs. Episodic lives in its own table at
`~/.linggen/memory/memory.lancedb/episodic.lance`.

**Write the tier explicitly when adding to core:**

```bash
ling mem add "<content>" --type fact --from user --tier core
ling mem list --tier core --limit 100 | jq -c 'del(.vector)'
```

Omit `--tier` and the row lands **episodic** (the default per-turn
capture). Write `--tier semantic` explicitly for long-term rows.

**If a candidate doesn't clearly fit core or long-term but might matter
later → episodic** (omit the tier; staging, the dream pass sorts it
out). **Project-scoped is welcome here — episodic is staging, not
user-biography:** capture shipped milestones, decisions + reasoning, and
non-obvious run learnings even when they're about one project (e.g.
*"Shipped Linggen 1.0"*, *"Sanji docking: treat all cost-points
uniformly"*). The only hard drops: secrets, and content verbatim
re-derivable from a file the agent re-reads — store the *decision/learning
about* it, never the file body, and Memory never writes to
`<project>/AGENTS.md`, `CLAUDE.md`, source, or docs.

**Goals and projects → long-term, not core.** *"User is building Linggen
as an agent platform"* is a goal — `tier=semantic`, not `--tier core`. Core is about the person;
goals are about the work. Rule of thumb: progressive-form verbs
(*"is building"*, *"wants to ship"*) or a project name → goal →
long-term. Names the person (*"is Alex"*, *"lives in Shanghai"*) →
core.

## Durability — what's worth remembering

Three rules decide whether a candidate earns its place. Routing (core
vs long-term tier) is a separate concern — these rules answer only
**should this be saved at all?** Memory never writes to project files
(`AGENTS.md`, `CLAUDE.md`, code, docs); candidates that don't fit core
or long-term are dropped.

1. **Don't memorize what lives in workspace files.** The agent reads
   them when needed. Putting the same content in memory creates a stale
   copy.
2. **User-stated preferences need a confidence gate.** Save when the
   user is correcting agent behavior with commitment language and
   cross-project reach. Skip single architectural calls. Synthesize at
   retrieval, not extraction.
3. **User-only knowledge — record, then maintain.** Stamp ages relative
   to a date (*"as of 2026-04-27"*, not *"3 years old"*). Append at
   write; reconcile at read.

For the full rules, examples, and the mechanical-vs-semantic
maintenance split, **Read `references/routing-rules.md`** before making
non-trivial save decisions.

## Mid-chat save rules — silent HIGH-SIGNAL auto-save

When the user utters one of these in regular chat, save immediately. No
widget, no confirmation, no verbose reply — just save and continue.

1. **Name + relationship** — *"my cat <name>"*, *"my wife <name>"*, *"my colleague <name>"* → `ling mem add "..." --type fact --from user --tier core`. Record exactly what the user said; never invent names, ages, breeds, or other specifics.
2. **Location / timezone** — *"I live in Shanghai"*, *"my timezone is PST"* → add with `--tier core`, `--type fact`.
3. **Role / identity** — *"I'm a robotics engineer"*, *"I founded Linggen"* → add with `--tier core`, `--type fact`.
4. **Long-term goal / vision** — *"I'm building X as Y"* → add with `--tier semantic --type fact`, scoped to the directory it is about (or `--global` when it spans the person's work). **Do NOT** use `--tier core` — goals belong in the long-term tier.
5. **Commitment-language preference** — *"always X"*, *"never Y"*, *"from now on Z"* → add with `--type preference --from user --tier semantic --summary "<one line>" --indexed`, `--global` when it holds everywhere, else scoped to its directory. **Not** core (decided 2026-09-09): core is who they are, not how they want the work done — the always-on block stays tiny, and recall surfaces a rule when its subject comes up.

Detect these patterns semantically, not lexically — works in any
language. *"我的猫叫 …"*, *"以后别再 …"* trigger the same routing.

Skip activity descriptions, project-specific technical facts (drop —
the agent will read the code), inferred preferences, opinions without
commitment.

**Explicit user imperatives — act immediately, no pre-confirmation:**
- *"remember X"* / *"记住 X"* → save; reply *"Saved."*
- *"forget X"* → search + delete; reply *"Deleted: <content>."* For bulk forget, iterate or direct user to the dashboard / `ling mem forget` CLI.
- *"update X to Y"* → search + update; reply *"Updated."*

## Retrieval is visible — chip every fact you used

When you call a memory query and the result shapes your reply, surface
what you used **in the chat text**, with the age of each fact:

> 💭 From memory (3 months ago): User has a cat.
> 💭 From memory (2 months ago): User lives in Shanghai.

Use **relative time**, dim or warn on facts older than 12 months
*(may be stale)*, skip the chip for facts you didn't actually use. When
two rows on the same subject surface, reconcile in prose ordered by
timestamp — don't silently rewrite or delete.

## Listing & searching memory — single-call recipes

When the user asks to list, browse, or search memory — whether via a
slash command, natural language, or any other phrasing — follow these
recipes. **One call per request.** Do not iterate over types, do not
add speculative filters. Over MCP the same calls are `memory_search` with
`op:"list"` (list), a bare `query` (search) and `op:"get"` (one row); the
CLI forms are below.

| User intent (any phrasing) | Make exactly this call |
|:---|:---|
| List everything (`/linggen list`, *"show all memory"*, *"list memory records"*, *"what's in memory"*) | `ling mem list --limit 100 --format json \| jq -c 'del(.vector)'` — **no filters at all** |
| List one type (`/linggen list facts`, *"show my preferences"*, *"list decisions"*) | `ling mem list --type <type> --limit 100 --format json \| jq -c 'del(.vector)'` |
| Search by content (`/linggen search <q>`, *"do you remember <q>"*, *"what do you know about <q>"*) | `ling mem search "<q>" --limit 10 --format json \| jq -c 'del(.vector)'` |
| Single noun like `/linggen cat` or *"my cat"* | `ling mem search "<noun>" --limit 10 --format json \| jq -c 'del(.vector)'` — search, not list |
| Get a specific row by id | `memory_search {op:"get", id}` — CLI: `ling mem get <uuid> --format json \| jq -c 'del(.vector)'` |

**FORBIDDEN unless the user explicitly asked for them:**
- `from` — filters by origin (user / agent / derived). Almost no read query needs this.
- `outcome` — filters by positive / negative / neutral. Most rows don't carry an outcome at all.
- Empty strings (`id: ""`, `query: ""`, `since: ""`) — leave the field out entirely.
- Empty arrays (`types: []`) — leave the field out entirely.
- Iterating types — **do NOT** call list once per type. A single unfiltered `list` returns every row in one round-trip.

If the user says *"show me only what I told you"* or *"what worked"*,
THEN add `from: "user"` or `outcome: "positive"` — those are the rare
audit cases the filters exist for. Otherwise omit them.

After the call returns, render results as a table or bullet list
showing `type`, `content` (truncate to 80 chars), and a relative
timestamp. Skip the id unless the user is about to delete or update.

## When to search

Call a memory search **before answering** when the user's question
could connect to past preferences / decisions / gotchas:

- *"How should I handle X?"* — look for related preferences / decisions.
- *"What did we decide about Y?"* — search with `type: decision`.
- *"Remember when we…"* — direct retrieval.
- Recurring operational question — search the project context if you're in a project workspace.

Skip search when the user is asking factual / technical questions with
no user-specific angle (*"what does this function do?"*, *"explain this
error"*).

## Scope and index — where a row belongs

A row's `scope` is the absolute directory it is about. Null = about
the person (visible everywhere); core rows never carry one. Modelled on
CLAUDE.md: a session sees its directory and every parent.

- **Writing.** The host stamps `cwd` (the session cwd: the default
  scope, request only), `root`, `source_session` and `host` — never fill
  those by hand. At session start the host shows `Memory scopes here: skills,
  skills/lingjing, … (default: <cwd>)`; pass one as `scope` on
  `memory_add` when the row is about another directory than where you
  stand (a 《九鼎录》 writing rule → `skills/lingjing`; "commit straight to
  main" → `~/workspace`). Memory accepts an existing dir inside the
  root or a parent of the root below `$HOME`, else falls back to the
  session cwd. `global: true` = about the person.
- **Index.** `indexed: true` puts the row into every session under its
  scope at start (nearest dir first, 3000-char budget): `## Index —
  <dir>` then `- summary (id=…)`. Set it, with a `summary` (one line,
  ≤ 80 chars, what the row is for), for standing rules the user states
  ("always…", "以后都…"). Without a summary the index shows the content's
  opening. An index line is a pointer — `memory_search {op:"get", id}`
  the row when it bears on the task. Rules already in a project file get a pointer summary
  (`写作规则见 DESIGN.md § 五·六`).
- **Moving.** `memory_update {"id","scope":"<abs dir>"}` (`~/` allowed)
  moves a row; `global: true` makes it about the person.
- **Recall scope.** A session in a project recalls rows under its root,
  at the root's parents, and about the person. A skill's own session
  (`~/.linggen/skills/<name>`) recalls only its own rows. `$HOME`,
  `~/.linggen` and temp dirs recall rows about the person, plus at most
  two strong matches filed under a directory — never a preference.
- **Merges.** A replacement keeps its losers' tier and their common
  scope. A digest is known by the rows whose `superseded_by` points at
  it — there are no tags.

## Modes — which references to load when

This skill enters one of two modes per invocation. **Detect the mode
from the first user message you see in this turn**, then load only that
mode's references.

| Mode | Detection cue (look at the first user message) | What to load |
|:---|:---|:---|
| **Dream** | Message says `/linggen dream` (all undreamed days) or `/linggen dream <YYYY-MM-DD>` (one day). User-triggered — or wired to the host's own scheduler for a nightly pass. | `Read references/dream-flow.md` (the canonical remember/forget runbook) and `references/routing-rules.md`. |
| **Scan** | Message says `/linggen scan <YYYY-MM-DD>` — stage that day's session logs (backfill), see the verb table. | `Read references/scan-flow.md` (the scan runbook) and `references/extractor-prompt.md` (what to stage). |
| **Solve** | Message says `/linggen solve` — drain the review queue (the rare leftovers a dream could not settle itself). | `Read references/solve-flow.md` (the solve runbook); `references/routing-rules.md` for write decisions. |
| **Status** | Message says `/linggen status` — one glanceable block: versions + updates, store size, upkeep. | Nothing extra: the host command carries the full recipe (fetches + render); its data = `memory_upkeep` op=dream_status + `ling mem stats` + engine/bridge probes. |
| **Chat** | **Anything else** — bare `/linggen`, `/linggen list`, `/linggen search foo`, plain `"show all memory"`, free-form questions. | Body of this SKILL.md is the entry. `Read references/routing-rules.md` only when making save / dedup decisions. |

**Chat mode is the default.** When in doubt, you are in chat mode.

## Slash commands — `dream` + memory passthrough

`/linggen <verb>` is the primary surface. `dream` is the
memory-consolidation pass (it runs the zero-LLM scan walk itself as
Phase 0, then judges); the rest map 1:1 to memory's CRUD verbs.
**`dream` is the headline verb**: it's the only one where the LLM does
judgment, and it's what a bare `/linggen` greeting should mention
first.

| Verb | Action |
|:---|:---|
| `dream` | **Remember all undreamed days, oldest first, then sweep.** Worklist via `ling mem days --undreamed`; per day: list its episodic rows → cluster → promote durable signal to semantic → `ling mem remember-day` stamp. Never deletes; the final `ling mem sweep` ages out judged rows past TTL, then the audit settles stale long-term rows itself from evidence and queues only what needs the user. See `references/dream-flow.md`. |
| `dream <YYYY-MM-DD>` | **Remember one day.** Same procedure, one day. |
| `scan <YYYY-MM-DD>` | **Stage one day's session logs (backfill).** See `references/scan-flow.md`. In Linggen itself a day can also be scanned from Settings → Memory. |
| `add "<content>" [--type ...] [--tier ...] [--scope <dir>] [--summary "..."]` | Insert a new memory row. Omitted tier = episodic. |
| `search "<query>" [--limit N]` | Semantic search across `semantic` + `episodic`. |
| `list [--type ...] [--tier ...] [--limit N]` | Paginated listing. |
| `delete <id>` | Remove a specific row by id. |
| `update <id> --content "<new>"` | Edit a row in-place (content / summary / indexed / scope). |
| `solve` | **Drain the review queue** — see `references/solve-flow.md`. |
| `status` | **Glanceable install status** — binary versions + cached update probes, store size (`ling mem stats`), and upkeep: `scanned_days`/`dreamed_days`/`total_days` counts, `first_unscanned` / `first_undreamed`, open issues, last run (from `memory_upkeep` op=dream_status or `ling mem days`). |

### Solve — `/linggen solve`

Drains the review queue: the rare leftovers a dream could not settle
itself. You are the solver, in the main chat, with this session's
model, tools and user. The procedure is `references/solve-flow.md` —
read it when solving, not before.

### Chat-mode rules

The user is reading text in a conversation panel:

- Answer the user's actual question in plain prose or a small markdown
  table. If the user asked to list memory, run the recipe in
  *Listing & searching memory* above and render the result inline.
- For hands-on row-level CRUD, point the user at Linggen's web UI
  (`http://127.0.0.1:9527` on the machine that runs Linggen).

## Memory hygiene — see it, solve it

**Hard rule, applies everywhere (live chat, per-turn capture, dream):**
whoever surfaces garbage owns it in that moment — **resolve it in the
same pass, don't defer**. There is no cleanup queue. Garbage in memory
poisons every future retrieval; "leave it for later" is how 7
word-count rows accumulate.

**Merge authority follows voice.** Your own notes (`from=derived` —
`built`/`fixed`/`tried`/`learned`) are your notebook: merge, rewrite,
retire freely, no prompt. Rows in the user's voice (`from=user` —
preference/decision/identity) change only with the user: ask first.
Memory enforces this floor mechanically — a replace or content
rewrite of a `from=user` row is BLOCKED unless the write carries
`user_directed: true`, which you assert only when the user directed
the change: their current message states it as settled (a command
"update X to Y", a declaration "my X is now Y", a commitment "from
now on, X") or they just answered your ask. A hedged reflection ("X
feels about right to me") never qualifies — ask first.

**Status rows are perishable — supersede at write time.** A
status-bearing row ("in progress", "OPEN:", "not committed",
"shipped", "dormant") is a claim about the world, and the world moves.
When you capture a status change (shipped / fixed / dormant /
abandoned), search the subject first and write the new status
replacing the prior status row(s) on that subject (`replace_ids` over
MCP; `add --replace <id>` via CLI) — never leave "in progress" beside its
own outcome. Own-notes only; a user-voice predecessor follows the
merge law. The dream audit settles what slips through (queueing only
what needs the user) — write-time supersede is the real fix.

| You see | Action |
|:---|:---|
| Exact dup (same fact, same type) | Delete the loser, keep the better-phrased row. No prompt. |
| Superseded / chain member, all derived ("impl not started" → "shipped") | Merge into one current-truth row. No prompt. |
| Reworded derived near-dup | Merge, keep the best phrasing. No prompt. |
| Old pure-event row ("committed X") | Retire it — fold into the state row it evidences, if one exists. |
| Contradiction touching a user-voice row | Don't pick silently. **Always ask.** |
| Secret (credential, token, key) | Delete on sight, any tier. |
| Judged episodic rows lingering past TTL | Run `ling mem sweep` — it evicts exactly those, never un-judged rows. No prompt. |

**How to ask:** use whichever ask-user primitive your host gives you.

- **Claude Code** — call the `AskUserQuestion` tool. UI renders a
  structured choice card.
- **Codex / any host without a structured tool** — write the
  question in plain chat text with numbered options and stop. The user
  replies on the next turn; you read their choice and finish the cleanup
  via `ling mem add "..." --type ... --replace <loser-id>` (one
  `--replace` per loser; add `--user-directed` after their answer).

When a merge (derived rows) or an AskUser-resolved conflict yields a
winner: write it with `ling mem add "<winner>" --type <t>
--from <f> --replace <loser-id>` — one atomic call; the losers are
archived, not deleted (over MCP: `replace_ids`).

### What "not confident" looks like

- Two rows on the same subject with timestamps far apart → user's view may
  have changed. Ask.
- Two rows that are mostly the same but differ on a specific detail (e.g.
  one says "8 years old in 2026-05-21", another says "9 years old in
  2026-05-25") → time-stamped, may both be valid. Ask before merging.
- Rows that look like dups but have different `scope` or
  `outcome` — they may apply to different directories. Ask.

When in doubt, **ask**. Cheap. The cost of asking is one turn; the cost of
silently losing or mangling a fact is much higher.

### What automatic catches mechanically

- `insert_with_dedup` inside the binary rejects byte-identical
  `(content, type)` rows at write time. You don't need to handle that case.
- Cross-tier dedup (`add` handler): if you add to one table and an exact
  match exists in the other, the higher-tier row wins and keeps its own
  scope; an empty summary or scope fills from the new write. Also automatic.

Fuzzy "same fact, different wording" is **never mechanical** — it always
needs an LLM judgment + the rule above.

### Inline reconciliation

When recall hits include duplicates or conflicts, fix them:
`ling mem delete <id>` near-dups (keep the best phrasing);
`ling mem edit <id>` or `delete` on conflicts after asking the user.
Get ids via `ling mem search "<phrase>" --format json | jq -r '.[] | "\(.id)\t\(.content)"'`.

## Type taxonomy (reference)

The `type` enum is `fact | preference | decision | tried | fixed |
learned | built` — but **only four should be emitted by default**.

| Type | Use | When to emit |
|:---|:---|:---|
| `fact` | Stable user truth (identity, goals, vision) | Cross-project, durable indefinitely |
| `preference` | Cross-project behavioral rule for the agent | Commitment language required |
| `decision` | A choice plus its reasoning | Reasoning is the retrieval value |
| `learned` | Cross-project tech gotcha | Reusable across projects |

`tried` / `fixed` / `built` are deprecated — emit only for
trajectory-level patterns or named shippable artifacts tied to user
identity.

## Updates

Linggen updates as one program: `ling update` (it keeps the previous
binary as `ling.prev` and restarts the running engine on the new one);
Linggen.app updates itself. `/linggen:status` shows when a newer release
is out. Don't update silently — ask first; behavior may change between
versions, and the user should know what they're accepting.

---

## Install

Install from your agent's own marketplace — it manages updates and, on
Claude Code / Codex, the per-turn recall hook. Pick **one** channel per host:

```text
Claude Code   claude plugin marketplace add linggen/linggen --sparse .claude-plugin plugins
              claude plugin install linggen@linggen
Codex         codex plugin marketplace add linggen/linggen
              codex plugin add linggen@linggen
Any MCP agent  add http://127.0.0.1:9527/mcp as an MCP server (tools only;
               another machine's Linggen shows it the five memory tools)
```

Linggen (`ling`) is installed on first use (SHA-256 verified) by the
plugin's session-start hook, or by `scripts/bootstrap.sh` on channels
without hooks (path relative to this skill's directory). Apple Silicon
Macs and Linux x86_64 / aarch64; Intel Macs are not supported yet.

The skill works in Claude Code, Codex, Linggen, or standalone —
one Linggen, one store, same semantics across all hosts.

Source: [github.com/linggen/linggen](https://github.com/linggen/linggen/tree/main/plugins/linggen) · [linggen.dev](https://linggen.dev)

## Delegating work to Linggen

Use the Linggen engine's MCP agent tools for delegated coding work:

1. Create a git worktree of the repo; pass its path as `write_dir` to `agent_run`.
   Linggen has admin there and read-only access elsewhere; beyond that it asks you.
2. Pass your current `permission_mode`: `plan`, `default`, `acceptEdits`,
   `auto`, or `bypassPermissions`. It maps your grants, never admin on all paths.
3. Send a self-contained brief: files, cause, tests to run, and "reply short".
   Only the final reply returns here; working steps stay in Linggen's web UI.
4. A long run continues in the background. Don't poll: start a background
   watcher (e.g. Claude Code's Monitor, ws source) on the
   `ws://127.0.0.1:9527/api/agent/wait?session_id=…` URL it returns. Frames:
   `{"type":"permission_request","request":{id,tool,summary,level,path}}` —
   answer each with `agent_permit` (`allow_once`, `allow_session` for that
   path/folder in this session only, or `deny`; none in 10 min = deny); last
   `{"type":"finished","status","reply","followup","error","model","cannot_run"}`,
   then it closes. If the watch closes before `finished`, call
   `agent_status {op:"session", session_id}` once (its `pending` lists open
   requests) and, if still running, watch again — never poll.
5. Read `denied` for refused actions and why; do the step yourself, or start
   a new run with a wider `write_dir`. Follow up with `agent_run(session_id=…)`;
   it keeps context. Omit `write_dir` and `permission_mode` on follow-ups.
6. Review the diff before merging. Finish with `agent_end {op:"close",
   session_id, remove_worktree:true}`; it removes the worktree only if clean
   and its HEAD is on a remote. `agent_end {op:"stop", session_id}` cancels a
   running turn. `agent_status {op:"models"}` lists every model and whether
   it is ready; `model` selects one for a new session.
7. A run that cannot go on says why in `cannot_run` (`reason`, `model`,
   `resets_at`, `line`). On a usage limit, read the reset time, then pick a
   ready model (`agent_status {op:"models"}`) or come back after it — never
   retry before. A run that reached its step limit says `step_limit`: the job
   was long, not the request wrong — continue with the same `session_id`.

## Browser control (via the same MCP server)

The `linggen` MCP server also exposes the user's own Chrome (through the
Linggen browser extension) — one **visible** controlled tab:

- Three tools, each with an `op`: `browser_open` (`navigate`, `tab_open`),
  `browser_read` (`page` — accessibility tree with `[nN]` refs —,
  `screenshot`, `console`, `tabs`) and `browser_act` (`click`, `type`,
  `key`, `scroll`, `wait`, `tab_switch`, `tab_close`). Work a read → act →
  re-read loop; target by ref, and read again after the page changes.
- Mutating actions may pause on a **permission prompt in the browser** —
  the user approves each new site once (or always); payment, credentials,
  deletes, and posting always confirm. A `not_permitted` error means the
  user declined: stop, don't retry.
- A `no_bridge` error means the Linggen browser extension isn't connected;
  ask the user to install or enable it.
- If the `linggen` MCP server itself is unreachable (nothing listening on
  `127.0.0.1:9527`), Linggen may still be installing in the background
  (plugin channels; progress in `~/.linggen/engine-install.log`) — wait and
  retry. If the `ling` binary is genuinely absent, run the install from the
  First-use section and tell the user (one-time, ~230 MB on Mac). Memory is served
  by the same program, so it comes back with it.

## The person's apps (via the same MCP server)

The `app` tool does what the person's apps do on their Mac — "play some jazz"
plays it. `op=list` names each app and its actions (with `app`, an action's
arguments); `op=call` with `app`, `action` and `args` runs one. A destructive
action is refused: tell the person to do it in the app. A private app answers
only once the owner opens it in Linggen's Settings → Tools & MCP; say so, and
do not retry.
