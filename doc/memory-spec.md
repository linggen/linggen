---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge across sessions — about the user, their work, and what the agent has done. Memory must help Linggen work better for every kind of user (software engineer, musician, language learner, cook), not just coders.

## Related docs

- `skill-spec.md`: how skills declare `provides:`, `tools:`, `daemon:`, `app:`, and `install:` — the manifest plug-points the memory skill uses.
- `session-spec.md`: system prompt assembly, `include_memory` profile flag.
- `storage-spec.md`: filesystem layout under `~/.linggen/`.
- `tool-spec.md`: built-in tools, skill-declared tools, capability-routed dispatch.
- `permission-spec.md`: tool permission tiers (declared by the skill manifest).

## Core principle

Memory has two layers:

1. **Built-in** (owned by Linggen) — a small pair of markdown files capturing universals about the user. Read and inlined into the stable system prompt every session.
2. **Skill memory** (pluggable) — a skill that advertises `provides: [memory]` and serves the `Memory.*` tool family over HTTP. Default skill: `linggen-memory` (a daemon backed by LanceDB). Handles facts, semantic retrieval, data UI.

The built-in layer is stable and minimal. Skill memory is where the work happens — and users can swap providers without touching Linggen itself.

## Layer 1 — Built-in (Linggen core)

### Files

Two markdown files under `~/.linggen/memory/`:

| File | Purpose | Contents |
|:-----|:--------|:---------|
| `identity.md` | Who the user is | Name, role, location, timezone, language, core preferences |
| `style.md` | How they want to be assisted | Tone, format, pacing, universal do/don't rules |

Both must be **universal** — true in any context, any project, any domain. If a fact wouldn't still matter in a totally different project six months from now, it doesn't go here.

### Loading

Linggen reads both files and inlines their content into the stable system prompt (the cacheable prefix). Files are re-hashed each turn, so turn-by-turn caching is preserved and user edits invalidate the cache on the next turn.

### Writing

- The user can edit either file directly (any editor — they're plain markdown).
- The agent edits them via the standard `Edit` / `Write` tools when the user explicitly asks to remember something universal. No dedicated "built-in memory" tool exists; the agent uses the same file-editing tools it uses everywhere, and the same path-permission plumbing applies.
- A daily consolidation mission (run by the active memory skill) can propose additions for user review.

The built-in layer is deliberately tiny (~30–50 lines combined). High bar for entry — no activity logs, no project-specific rules, no meta-feedback about how memory should work.

## Layer 2 — Memory skill (pluggable)

### The `provides: [memory]` contract

A skill becomes the active memory provider by declaring `provides: [memory]` in its `SKILL.md` frontmatter. Linggen detects the capability on skill load, adds the declared tools to its registry, and routes `Memory.*` calls to that skill's daemon.

Only one memory provider is active per session. If multiple are installed, resolution is deterministic (see `skill-spec.md` for capability arbitration rules) and the user can override via config.

### The `Memory_*` tool family (canonical contract)

The engine defines **exactly these seven tools** in `engine::capabilities`. Schemas, arg names, types, and permission tiers are engine-owned:

| Tool | Purpose | Tier |
|:-----|:--------|:-----|
| `Memory_add` | Insert a fact. Args: `content`, `contexts[]`, `type`, optional metadata. | Edit |
| `Memory_get` | Fetch a single row by id. | Read |
| `Memory_search` | Semantic search. Args: `query`, optional filters. | Read |
| `Memory_list` | Browse/filter without semantic ranking. | Read |
| `Memory_update` | Edit an existing row. | Edit |
| `Memory_delete` | Hard-forget a single row. | Edit |
| `Memory_forget` | Bulk-delete by filter. | Admin |

**Engine owns the contract, skills implement it.** A memory skill declares `provides: [memory]` and an `implements:` block mapping each tool name to its HTTP endpoint on the skill's daemon. It does **not** re-declare the argument schema — the engine already has it. This keeps the model's experience identical across providers: swapping skills changes where data lives, not what tools exist.

These tools appear in the model's tool list only when:
1. The session's prompt profile opts into memory (`include_memory: true` — owner sessions do; consumer / mission sessions don't).
2. An active `provides: [memory]` skill is installed.

If no memory skill is installed, the capability is inactive and Linggen filters the tools out of the model's tool list entirely. The model doesn't see them at all.

### Plug-points in one manifest

The memory skill opts into several `SKILL.md` plug-points at once — each one extends a different engine subsystem. See `skill-spec.md` for the full plug-point model.

| Plug-point | What it does for this skill |
|:-----------|:----------------------------|
| `name:` + body | `/memory` slash command loads the skill body as session context |
| `provides: [memory]` | Claims the memory capability |
| `implements: memory:` | Binds the engine's canonical `Memory_*` tools to this daemon's endpoints |
| `app:` | Exposes the skill's dashboard UI as a Linggen app |
| `install:` | Downloads the `ling-mem` binary, places mission files under `~/.linggen/missions/memory/` |

The `implements:` block is the **only** place the skill talks about tool routing. It does not include arg schemas — those live in the engine. See `skill-spec.md` for the full `implements:` syntax.

### HTTP dispatch contract

When a `Memory_*` tool is called, the engine routes to the active provider's daemon over HTTP.

1. **Capability lookup.** The engine resolves the tool name to its capability (`Memory_search` → `memory`) via its capability registry.

2. **Provider lookup.** The engine asks its skill manager for the active `provides: [memory]` skill. Exactly one provider is active at a time (see `skill-spec.md` § Capability arbitration).

3. **URL construction.** The engine reads the provider's `implements.memory` block, takes `base_url`, looks up the tool's path in `tools[tool_name]`, and concatenates. Example: `http://127.0.0.1:9888` + `/api/memory/search` → `http://127.0.0.1:9888/api/memory/search`.

4. **POST.** The engine POSTs the tool's JSON args as the request body — no flag translation, no schema re-mapping.

5. **Response envelope.** Success: `2xx` with `{ok: true, data: <value>}`. The `data` shape is whatever the method returns — a single object for `get`/`add`/`update`, an array for `search`/`list`, `null` for `delete`/`forget`. Error: non-2xx with `{ok: false, error: "...", code: "..."}`. The engine surfaces errors to the model as `provider error [CODE]: MSG`.

6. **Autostart, never auto-stop.** On a connection refuse or timeout on the first attempt, the engine spawns the provider's `implements.memory.autostart` command (default: `ling-mem start`) and retries once. The engine **does not** auto-stop the daemon — it outlives the Linggen process. Users manage shutdown explicitly (`ling-mem stop`) or via OS-level service managers.

7. **Network scope.** Daemons bind `127.0.0.1` only. The `Memory_*` tools are exposed only to owner sessions (never to consumers or missions), so every call originates from the local Linggen process. Remote access is out of scope; auth is deferred until a multi-user-on-one-box scenario materializes.

8. **Per-provider isolation.** Each memory skill stores data under `~/.linggen/memory/<skill-name>/`. Swapping providers leaves the previous provider's data on disk; exports/imports move data between them.

### Per-provider data layout

Under `~/.linggen/memory/<skill-name>/`:

- `data/` — provider-internal store (LanceDB files for `linggen-memory`)
- `logs/` — daemon logs
- `config.toml` — optional user overrides (e.g. non-default port)
- `daemon.json` — pidfile written by the daemon for its own `ling-mem status` command (the engine doesn't consult it; dispatch uses the `base_url` from the manifest directly)

The exact layout inside `data/` is provider-internal.

### Default skill: `linggen-memory`

Shipped as Linggen's default memory provider. Lives in its own repo (`linggen-memory/`), released as a platform-specific binary on GitHub. The skill package in `skills/memory/` is a thin wrapper: `SKILL.md` + `install.sh` (downloads the binary).

- **Runtime model:** HTTP daemon (`ling-mem start`). All data operations go through the daemon; there is no per-call subprocess.
- **Storage:** LanceDB — vector + metadata, semantic search.
- **Retrieval:** Hybrid BM25 + vector similarity, filterable by context/type.
- **Data UI:** embedded webpage served by the daemon at its port (default `http://127.0.0.1:9888`). Row-level editor, filter/sort, batch archive/forget. Pure data browser — no missions, no extraction, no chat widget. Opening the UI never triggers a mutating task.
- **Lifecycle CLI:** `ling-mem start | stop | restart | status | version`. Data operations are **not** CLI-accessible — they're HTTP-only. Power users querying from a terminal use `curl`.
- **Install:** platform-aware `install.sh` — `uname` detection, download matching release asset, fallback to `cargo install` for unknown platforms.
- **No model, no agent.** `linggen-memory` is a data service. Anything that needs reasoning (extraction, consolidation, summary) lives in Linggen, not in the daemon.

Users who prefer a different memory strategy can write their own skill that conforms to `provides: [memory]` with the seven canonical tools. Linggen is neutral about the implementation.

### Two UIs, one package

The memory skill presents two separate UI surfaces — decoupled, each with a single responsibility:

- **Data UI** (served by the daemon, default `http://127.0.0.1:9888`) — pure CRUD over the LanceDB store. Filter, edit, archive, forget. No side-effects on open.
- **Skill dashboard** (served by Linggen as an `app:` skill) — summary cards, extraction mission controls, chat widget for the memory agent. Deep-links into the data UI when the user wants row-level editing.

Opening either surface never triggers a mutating task (extraction, rebuild, etc.). Mutations are always explicit actions — a button click, a `Memory.*` tool call, or a scheduled mission firing.

## Data model (default skill)

The LanceDB schema is owned by the `linggen-memory` skill. The locked shape lives in [linggen-memory/DESIGN.md](../../linggen-memory/DESIGN.md). This spec does not duplicate it.

Key points for a Linggen integrator:

- **Row identity is a UUID**, not a path or filename. Linggen never constructs or parses row ids.
- **Scoping is via free-form `contexts[]`** (e.g. `code/linggen`, `music/piano`, `trip-japan-2026`). Contexts are N:M tags, not directory paths — one fact can span multiple contexts.
- **`type` is a closed enum** with seven canonical values. Linggen validates nothing about types — it passes whatever the model chose through and lets the skill reject or coerce.
- **Embedding and ranking are skill-internal.** Linggen never computes vectors or scores; it just forwards queries and reads results.

Any schema drift between this document and `linggen-memory/DESIGN.md` is a bug in this document; `DESIGN.md` wins.

## Retrieval patterns

Three access modes, all backed by the active memory skill:

1. **Push (active injection).** Linggen calls `Memory.search` with the user's message at turn start and prefixes matched snippets to the user message. Runs per turn. Cache-safe (doesn't invalidate the stable system prompt).
2. **Pull (tool).** The model calls `Memory.search` / `Memory.list` when it decides memory would help. Standard tool dispatch.
3. **Browse (UI).** The user opens the daemon's data UI for row-level review, edit, archive, forget — or uses the memory skill's dashboard UI for a higher-level summary with extraction controls.

Built-in files are always inlined — they're small enough that inlining beats querying.

## Extraction

Extraction — turning session transcripts into `Memory.add` calls — is driven by **Linggen**, not by the memory skill's daemon. The daemon has no model and no agent; it's a data service.

The memory skill ships a mission file in its `assets/` directory; `install.sh` copies it to `~/.linggen/missions/memory/` (following the pattern in `skill-spec.md`). Linggen's mission scheduler picks it up and fires the extraction agent on schedule. The agent reads session transcripts and makes `Memory.add` calls as it finds durable facts.

The skill owns the *prompt* and the *cadence*; Linggen owns the *runtime* (scheduling, agent, LLM). Disabling extraction is removing the mission file or turning off its schedule — no daemon-side knob involved.

## Forgetting

Forgetting is skill-internal — Linggen provides `Memory.forget` for bulk-delete-by-filter, but decay policy (time-based, access-based), durability filters at write time, and any compaction passes are all owned by the active memory provider. See `linggen-memory/DESIGN.md`.

Linggen's only contract: when the user explicitly says "forget everything about X," the model calls `Memory.forget` with the matching filter and trusts the result.

## Mid-session self-review

Linggen fires a hidden nudge every N user messages (configurable via `[agent] memory_nudge_interval`, default 6, 0 disables). The nudge asks the model whether the recent exchange produced anything worth saving — either an `Edit` to `identity.md` / `style.md` for universals, or a `Memory.add` call for scoped facts. Gated behind `include_memory` like the rest of memory.

The nudge text lives in the active skill's manifest (under `prompts.nudge`) so alternate providers can customize the wording. If the active skill doesn't declare a nudge, Linggen falls back to a generic default.

## Invocation surface

The memory skill is a composite package — one `SKILL.md` reached via multiple paths:

- **User types `/memory`** → skill body is loaded as session context. The model "thinks in memory mode" for that session and can use `Memory.*` tools, open the dashboard, start extractions, etc. But the slash command is not required — the tools are ambient.
- **Model calls `Memory.search`** → Linggen dispatches to the daemon over HTTP. Works in any owner session, regardless of whether `/memory` was invoked.
- **Model calls `RunApp memory-dashboard`** → the skill's dashboard UI opens in a panel.
- **Mission scheduler fires** → the extraction agent runs, populates memory via `Memory.add`.

Tool dispatch always routes to the `provides: [memory]` **active provider**, not to whichever skill was named in a slash command. If two memory skills are installed and the user invokes `/memory-alt`, the `Memory.*` tools still hit the ambient active provider. The slash command frames the model's attention; the data plane remains anchored to the active provider.

## Fresh build — no migration

The v1 5-markdown-file system (`user_info.md`, `user_feedback.md`, `agent_done_{week,month,year}.md`) is retired without migration. Users start empty and populate `identity.md` / `style.md` on demand. The v0.1 CLI-per-call dispatch is also retired — all memory operations now go through the HTTP daemon. Anything under `~/.linggen/memory/` besides the two built-in files is data owned by the installed memory skills.

## Safety

| Guard | Rationale |
|:------|:----------|
| No secrets | Never store credentials, API keys, tokens, passwords — at any layer |
| Built-in is read-first | Agent writes to `identity.md` / `style.md` only on explicit user request |
| Schema-versioned data | Every stored row carries a schema version; migrations are explicit |
| Human-readable surface | Built-in layer is markdown. Skill rows are browsable via the daemon's data UI. Nothing is opaque. |
| Export to markdown | Default skill nightly-exports LanceDB to markdown for backup/git-sync |
| Durability filter | The active memory skill decides what's durable before committing — not Linggen |
| Localhost-only bind | Daemons bind `127.0.0.1`; `Memory.*` never exposed to remote consumers |
| Per-provider isolation | All data under `~/.linggen/memory/<skill-name>/`; swapping providers keeps old data intact |
| Capability-routed dispatch | Swapping memory skills is one config setting, no data loss |
| No side-effects on UI open | Opening any memory UI never triggers a mutating task; mutations are always explicit |

## Future

- **Cross-device sync** — LanceDB exports + git is v1; real sync is P2P via Linggen's WebRTC transport.
- **Temporal tracking** — record how facts change over time (inspired by Zep). `supersedes` links already support this structurally.
- **Multi-provider** — use a local fast skill + a cloud/persistent skill simultaneously, with merged results. Design TBD.
- **`Memory.archive`** — soft-forget (hidden from default search but recoverable). Eighth tool to add once the default skill supports it; canonical contract will extend to eight.
- **Memory health scoring** — auto-detect and propose cleanup for degraded memories.
