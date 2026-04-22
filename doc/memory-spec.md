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

- `skill-spec.md`: `provides:` capability, skill discovery, install hooks.
- `session-spec.md`: system prompt assembly, memory injection layer.
- `storage-spec.md`: filesystem layout under `~/.linggen/`.
- `tool-spec.md`: built-in tools (including `Memory.*`).

## Core principle

Memory has two layers:

1. **Core** (built into Linggen) — a small set of markdown files capturing universals about the user. Read and injected on every session.
2. **Skill memory** (pluggable) — a skill that advertises `provides: [memory]` and implements the `Memory.*` tool family. Default skill: `linggen-memory` (Rust binary backed by LanceDB). Handles facts, activity, semantic retrieval, edit UI.

The core layer is stable and minimal. Skill memory is where the work happens — and users can swap providers without touching Linggen itself.

## Layer 1 — Core (built-in)

### Files

Two markdown files under `~/.linggen/core/`:

| File | Purpose | Contents |
|:-----|:--------|:---------|
| `identity.md` | Who the user is | Name, role, location, timezone, language, core preferences |
| `style.md` | How they want to be assisted | Tone, format, pacing, universal do/don't rules |

Both must be **universal** — true in any context, any project, any domain. If a fact wouldn't still matter in a totally different project six months from now, it doesn't go in core.

### Loading

Linggen reads both files on startup and inlines their content into the stable system prompt (the cacheable prefix). They re-hash only when the files change on disk, so turn-by-turn caching is preserved.

### Writing

- The user can `vim` either file directly.
- The agent can Edit them in-session when the user explicitly asks to remember something durable.
- A daily consolidation pass (run by the active memory skill) can propose additions for user review.

Core is deliberately tiny (~30–50 lines combined). High bar for entry — no activity logs, no project-specific rules, no meta-feedback about how memory should work.

## Layer 2 — Memory skill (pluggable)

### The `provides: [memory]` contract

A skill becomes the active memory provider by declaring this in its `SKILL.md` frontmatter. Linggen core detects the capability on skill load and routes `Memory.*` tool calls to that skill's handler.

Only one memory provider is active per session. If multiple are installed, the user selects (or Linggen picks one by install order — see skill-spec.md for resolution rules).

### The `Memory.*` tool family

Linggen registers these tools automatically when a memory provider is active:

| Tool | Purpose |
|:-----|:--------|
| `Memory.add` | Insert a fact. Args: `content`, `contexts[]`, `type`, optional metadata. |
| `Memory.search` | Semantic search. Args: `query`, optional `contexts`, `type`, `limit`. |
| `Memory.list` | Browse/filter without semantic ranking. Args: filters, sort, pagination. |
| `Memory.get` | Fetch by id. |
| `Memory.update` | Edit an existing fact. |
| `Memory.archive` | Soft-forget — hidden from default search but recoverable. |
| `Memory.delete` | Hard-forget with tombstone. |
| `Memory.forget` | Bulk-delete by filter. Used for "forget everything about the trip." |

These tools are gated behind the session's `include_memory` profile flag. Consumer and mission sessions do not see them.

If no memory skill is installed, the tools return a clear error ("No memory provider active; install one from the marketplace").

### Default skill: `linggen-memory`

Shipped as Linggen's default memory implementation. Lives in its own repo (`linggen-memory/`), built as a platform-specific binary released on GitHub. The skill in `skills/memory/` is a thin wrapper: `SKILL.md` + `install.sh` that downloads the binary.

- **Storage:** LanceDB — markdown-free, vector + metadata, semantic search.
- **Retrieval:** Hybrid BM25 + vector similarity, filterable by context/type.
- **Edit UI:** embedded webpage served by the binary in daemon mode. Markdown-like editor per row, filter/sort, batch archive/forget.
- **CLI:** `linggen-memory add|search|list|update|archive|delete|forget|collect|extract|serve`.
- **Install:** platform-aware `install.sh` — `uname` detection, download matching release asset, fallback to `cargo install` for unknown platforms.

Users who prefer a different memory strategy can write their own skill that conforms to `provides: [memory]` and implements the `Memory.*` handler contract. Linggen is neutral about the implementation.

### Binary-invocation contract (locked)

When `Memory.*` is called, Linggen core locates and invokes the provider binary as follows. The contract here matches the `linggen-memory` v0.1 CLI locked in its `doc/tech-spec.md`.

1. **Locate the binary:**
   - First: `$SKILL_DIR/bin/ling-mem` — the skill's `install.sh` places the binary there.
   - Fallback: bare `ling-mem` on `$PATH` (for `cargo install` / dev setups).
   - If neither resolves at spawn time, dispatch surfaces a "binary not found" error pointing the user at the skill's install script.

2. **Invoke:** `ling-mem <method> [positional] [flags]`
   - `<method>` is the lowercase trailing component of the tool name — `Memory.search` → `search`.
   - Args from the JSON payload are translated into the CLI's positional + flag shape per method (e.g. `contexts: ["a","b"]` → `--context a --context b`). The translation table lives in `engine::memory::translate_args`.
   - `LINGGEN_DATA_DIR` is exported to the subprocess (per-user data root). The binary opens its LanceDB store underneath.
   - `delete` and `forget` are invoked with `--yes`; Linggen's permission layer already captured the user's consent.

3. **Response format:**
   - Exit 0 + JSON on stdout. A single object, a single array, or NDJSON (one object per line) all work — Linggen parses each shape into one `Value` returned to the tool caller.
   - Non-zero exit + stderr: structured JSON `{"error":"...","code":"..."}` is preferred and surfaced as `provider error [CODE]: MSG`. Raw text on stderr falls back to `provider error: <text>`.

4. **Timeouts:** 5-second default per call; the dispatcher kills the subprocess on timeout. Long-running ops belong in the provider's `serve` daemon mode, not in the sync CLI path.

## Data model (default skill)

The LanceDB schema is owned by the `linggen-memory` skill. The locked v0.1 shape lives in [linggen-memory/DESIGN.md](../../linggen-memory/DESIGN.md) — single source of truth. This spec does not duplicate it.

Key points a Linggen-core integrator needs to know:

- **Row identity is a UUID**, not a path or filename. Nothing in Linggen core constructs or parses row ids.
- **Scoping is via free-form `contexts[]`** (e.g. `code/linggen`, `music/piano`, `trip-japan-2026`). Contexts are N:M tags, not directory paths — one fact can span multiple contexts. Linggen core never assumes a 1:1 between context and project.
- **`type` is a closed enum** with seven canonical values. Linggen core validates nothing about types — it passes whatever the model chose through to `Memory.add` and lets the skill reject or coerce.
- **Embedding and ranking are skill-internal.** Linggen core never computes vectors or scores; it just forwards queries and reads results.

Any schema drift between this document and `linggen-memory/DESIGN.md` is a bug in this document; `DESIGN.md` wins.

## Retrieval patterns

Three access modes, all backed by the active memory skill:

1. **Push (active injection).** Linggen calls `Memory.search` with the user's message at turn start and prefixes matched snippets to the user message. Runs per turn. Cache-safe (does not invalidate the stable system prompt).
2. **Pull (tool).** The model calls `Memory.search`, `Memory.list`, etc. when it decides memory would help. Standard tool dispatch.
3. **Browse (UI).** The user opens the memory app to review, edit, archive, forget rows directly.

Core files are always injected — they're so small it's cheaper to inline than to query.

## Extraction

Extraction — turning session transcripts into `Memory.add` calls — is a skill-internal concern. Linggen core contributes only session transcripts on disk (`~/.linggen/sessions/*`) and the `Memory.*` tool family. The provider decides how, when, and what to extract.

For the default provider, see `linggen-memory/DESIGN.md` (Phase 3 adds `collect` + `extract` subcommands to `ling-mem`). The `project_root` available in each session transcript is the natural seed for the `contexts[]` tag — a session run under the Linggen repo becomes `contexts: [code/linggen]` — but nothing in this spec mandates that mapping.

## Forgetting

Forgetting is a skill-internal concern — Linggen core provides the `Memory.forget` tool for bulk-delete-by-filter, but decay policy (time-based, access-based), durability filters at write time, and any compaction passes are all owned by the active memory provider. See `linggen-memory/DESIGN.md` for that provider's approach.

Linggen core's only contract: when the user explicitly says "forget everything about X," the model calls `Memory.forget` with the matching filter and trusts the result.

## Mid-session self-review

Linggen core fires a hidden nudge every N user messages (configurable via `[agent] memory_nudge_interval`, default 6, 0 disables). The nudge asks the model whether the recent exchange produced anything worth saving — either an Edit to `identity.md` / `style.md` for universals, or a `Memory.add` call for scoped facts (when a provider is installed). Gated behind `include_memory` like the rest of memory.

## Fresh build — no migration

The v1 5-markdown-file system (`user_info.md`, `user_feedback.md`, `agent_done_{week,month,year}.md`) is retired without migration. Users start core empty and populate `identity.md` / `style.md` on demand. Anything from the old files that matters can be re-added by the user (or prompted via the self-review nudge) — we are not trying to reconstruct history.

Any existing files under `~/.linggen/memory/` are ignored by the Linggen core as of this version. They remain on disk until the user removes them. The memory skill's future `migrate` subcommand may offer an opt-in import, but that is a skill-owned concern, not a core behavior.

## Safety

| Guard | Rationale |
|:------|:----------|
| No secrets | Never store credentials, API keys, tokens, passwords — at any layer |
| Core is read-first | Agent writes to core only on explicit user request |
| Schema-versioned data | Every LanceDB row has a schema version; migrations are explicit |
| Human-readable surface | Core is markdown. LanceDB rows are browsable via the app. Nothing is opaque. |
| Export to markdown | Default skill nightly-exports LanceDB to markdown for backup/git-sync |
| Durability filter | The active memory skill decides what's durable before calling `Memory.add` — not Linggen core |
| Per-user isolation | All paths under `~/.linggen/memory/<user_id>/` when multi-tenant is active |
| Capability-routed dispatch | Swapping memory skills is one setting, no data loss (new provider starts empty; export/import moves data between) |

## Future

- **Cross-device sync** — LanceDB exports + git is v1; real sync is P2P via Linggen's WebRTC transport.
- **Temporal tracking** — record how facts change over time (inspired by Zep). `supersedes` links already support this structurally.
- **Multi-provider** — use a local fast skill + a cloud/persistent skill simultaneously, with merged results. Design TBD.
- **Memory health scoring** — auto-detect and propose cleanup for degraded memories (inspired by OpenClaw).
