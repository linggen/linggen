---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Memory

Persistent knowledge extracted from conversations. The agent remembers who the user is and what it has done — across all sessions, all projects.

## Related docs

- `session-spec.md`: system prompt assembly (layer 7 = memory).
- `storage-spec.md`: filesystem layout (`~/.linggen/memory/`).
- `skill-spec.md`: skill format, install hooks, app skills.
- `mission-spec.md`: missions trigger the memory skill nightly.

## Core concept

Memory has two parts:

1. **Loading** (built-in) — the engine reads memory file frontmatters at startup and injects descriptions into the system prompt. Same pattern as skill discovery.
2. **Writing** (skill) — a nightly mission runs the `memory` skill, which collects the day's sessions, sends them to the model one at a time, and the model updates the memory files.

All memory is **global** (`~/.linggen/memory/`). No project-scoped memory. If a user says "I prefer Rust over Java" in one project, every future session should know that.

Two units are tracked: **user** (what the user said, claims, prefers) and **agent** (what the agent did, decided, observed).

## Built-in vs skill

The split follows the same pattern as skills: **engine loads metadata, skill does the work.**

| Concern | Who | How |
|:--------|:----|:----|
| Read frontmatter descriptions at startup | **Engine (built-in)** | Scan `~/.linggen/memory/*.md`, parse YAML frontmatter |
| Inject descriptions into system prompt | **Engine (built-in)** | Layer 7, same as skill descriptions in layer 3 |
| Bootstrap empty state | **Engine (built-in)** | If no files exist, inject template with file format so model can create them |
| Create memory files from templates | **Skill install script** | `install.sh` copies templates from `assets/` to `~/.linggen/memory/` |
| Create nightly mission | **Skill install script** | `install.sh` copies `assets/mission.md` to `~/.linggen/missions/memory/` |
| Extract facts from conversations | **Skill (web app or mission)** | App collects sessions, sends each to model, model extracts and updates |
| Compress week → month → year | **Skill** | Same extraction run handles compression |
| Real-time "remember this" writes | **Agent** | Uses `Edit` tool directly, body only — never touches frontmatter |

The engine's built-in part is minimal: read frontmatter, inject descriptions, handle empty state. All extraction intelligence lives in the skill.

### Frontmatter is fixed

Memory file frontmatter (`name`, `description`, `unit`, `updated_at`, `retention`) is defined by the templates in `skills/memory/assets/`. The model **never edits frontmatter** — only the body content below the `---` delimiter. This prevents description drift and format corruption.

## Fixed memory files (v1)

Five well-known files. The memory skill only writes to these — no ad-hoc file creation in v1.

| File | Unit | Purpose |
|:-----|:-----|:--------|
| `user_info.md` | user | Identity, preferences, hobbies, claims — everything the user has ever told the agent |
| `user_feedback.md` | user | How the user wants the agent to behave — corrections, confirmed approaches, style rules |
| `agent_done_week.md` | agent | What the agent did this week — detailed, rolling 7 days |
| `agent_done_month.md` | agent | Compressed monthly summary — key actions and outcomes |
| `agent_done_year.md` | agent | High-level yearly summary — major milestones only |

### Time-decay model

Like human memory: recent events are vivid, older ones fade to what mattered.

```
This week          → agent_done_week.md   (detailed: files changed, commands run, decisions made)
  ↓ compress
Past months        → agent_done_month.md  (summarized: features built, bugs fixed, deploys)
  ↓ compress
Past years         → agent_done_year.md   (highlights: major milestones, architecture changes)
```

### Size guidelines

| File | Target size | Guideline |
|:-----|:------------|:----------|
| `user_info.md` | < 200 lines | Factoids grouped by section. One line per fact. |
| `user_feedback.md` | < 100 lines | Do/don't rules. One line per rule. |
| `agent_done_week.md` | < 150 lines | ~10-20 bullet points per day, curated not exhaustive |
| `agent_done_month.md` | < 200 lines | ~10-15 bullets per month |
| `agent_done_year.md` | < 100 lines | ~10-20 bullets per year |

## Memory file format

Each memory file is markdown with YAML frontmatter.

### Frontmatter fields

| Field | Required | Purpose |
|:------|:---------|:--------|
| `name` | yes | File identifier, matches filename without `.md` |
| `description` | yes | **Category summary** of what's inside (~150 chars). Describes the *kinds* of facts, not individual facts. Loaded into every session's system prompt. |
| `unit` | yes | `user` or `agent` — who this memory is about |
| `updated_at` | yes | Last modified date (YYYY-MM-DD) |
| `retention` | no | `week`, `month`, or `year` — for agent_done files, controls the compression tier |

The `description` field should describe **categories**, not enumerate facts:

```yaml
# Good — categories tell the model what's inside
description: "User personal info — identity, role, preferences, hobbies, pets, health, claims"

# Bad — tries to list facts, misses most of them
description: "Liang: developer, February birthday, dark mode, hiking"
```

## Storage layout

```
~/.linggen/memory/
  user_info.md
  user_feedback.md
  agent_done_week.md
  agent_done_month.md
  agent_done_year.md
```

Flat. Five files. No index file — the engine scans the directory and reads each file's frontmatter directly.

## Loading

Same progressive disclosure pattern as skills.

### Descriptions (loaded at startup)

The engine scans `~/.linggen/memory/*.md`, parses each file's YAML frontmatter, and holds `name` + `description` in memory. On each session's system prompt assembly (layer 7), descriptions are injected:

```
You have persistent memory files at ~/.linggen/memory/:
- user_info: "User personal info — identity, role, expertise, preferences, hobbies, pets, health, claims"
- user_feedback: "Agent behavior rules — workflow, style, communication, coding conventions, do/don't"
- agent_done_week: "Agent actions this week — files changed, features built, bugs fixed, deploys, decisions"
- agent_done_month: "Agent actions past months — features shipped, major fixes, architecture changes, deploys"
- agent_done_year: "Agent actions past years — major milestones, launches, architecture shifts"

Read the full file when a conversation needs the details.
After completing significant work, update agent_done_week.md.
```

~300 tokens. Refreshed when files change on disk (staleness detection hashes all memory files).

### Empty state

If no memory files exist (first run before install), the engine injects a bootstrap template that describes the 5-file format so the model can create them on demand. Once the skill's install script runs, the templates are in place and the normal description block is used.

### Full content (loaded on demand)

The agent uses `Read` to open the full memory file when the conversation needs it. No special tool — just the standard `Read` tool pointing at `~/.linggen/memory/*.md`.

## The memory skill

The memory skill is a **web app skill** — like sys-doctor, it has a JS dashboard that orchestrates extraction and visualizes results in real-time.

### Skill structure

```
skills/memory/
├── SKILL.md                    # Model instructions for extraction
├── assets/                     # Templates — source of truth for frontmatter
│   ├── user_info.md
│   ├── user_feedback.md
│   ├── agent_done_week.md
│   ├── agent_done_month.md
│   ├── agent_done_year.md
│   └── mission.md              # Nightly mission definition
└── scripts/
    ├── install.sh              # Copies templates + mission on install
    ├── collect_sessions.sh     # Scans CC + Linggen sessions
    └── index.html              # Web app dashboard (future)
```

### Install

The skill declares `install: scripts/install.sh` in its frontmatter. On install, the script:

1. Creates `~/.linggen/memory/` and copies the 5 template files (skip if exist)
2. Creates `~/.linggen/missions/memory/` and copies `mission.md` (skip if exists)

Idempotent — safe to run multiple times. Templates are the source of truth for frontmatter; existing files are never overwritten.

### Cross-tool session collection

The collection script (`scripts/collect_sessions.sh`) auto-discovers sessions from:

- **Claude Code** — `~/.claude/projects/*/*.jsonl` (ISO timestamps, content blocks)
- **Linggen** — `~/.linggen/sessions/*/messages.jsonl` (epoch timestamps, flat strings)

Safety guards:
- **Self-ingestion prevention** — skips Linggen sessions with `creator: mission` to prevent the memory extraction mission from re-processing its own output.
- **Deduplication** — Linggen sessions may already be reflected in memory (the agent was present and could have written in real-time). The model checks existing memory before adding.

### Extraction flow

The web app (or mission runner) orchestrates extraction:

```
1. Run collect_sessions.sh → individual session files
2. Create a skill-bound session
3. For each session file:
     Send session content to model
     Model extracts facts, updates memory files via Edit
     Dashboard shows live progress: found/skipped/merged
4. Model runs compression (week → month → year)
5. Dashboard shows report: changes by file, conflicts resolved, stale entries removed
```

The model receives **one session at a time** — small enough for any model size. The orchestrator (JS app or Python runner) controls the loop.

### Mission modes

The nightly mission supports two modes:

| Mode | How it runs | Use case |
|:-----|:-----------|:---------|
| **agent** | Scheduler creates session, model self-orchestrates | Simple, works with capable models |
| **script** | Scheduler runs a runner script, script calls APIs | Reliable, works with any model size |

In agent mode (current), the model reads SKILL.md and follows the extraction steps using tools. In script mode (future), a Python/Node runner drives the loop — same pattern as the JS dashboard, but headless.

### Web app dashboard

The memory dashboard follows the same pattern as sys-doctor:

- **Left panel**: dashboard widgets (profile card, behavior rules, weekly timeline, extraction progress)
- **Right panel**: chat with the memory agent
- **Page protocol**: model emits `<!--page {...} -->` JSON blocks, app renders widgets

Dashboard views:
- **Overview** — memory cards showing user profile, behavior rules, this week's actions, file sizes
- **Extraction** — live progress: sessions being scanned, facts found/skipped/merged, conflicts
- **Report** — post-extraction summary: changes by file, compression results, stale entries removed

## Real-time writes

The agent can also write to memory files during any conversation — not just during nightly extraction. Two cases:

1. **User explicitly asks** — "remember that I prefer dark mode" → agent reads existing file, edits body via `Edit` tool.
2. **Agent completes significant work** — after a major feature or deploy, appends to `agent_done_week.md`.

The nightly mission is the safety net — it catches what the agent missed in real-time.

## Safety

| Guard | Rationale |
|:------|:----------|
| No secrets | Never store credentials, API keys, tokens, passwords |
| Frontmatter is fixed | Templates are source of truth — model only edits body, never frontmatter |
| Record user claims as-is | No fact-checking — but label unverified claims |
| Human-readable | User can inspect, edit, or delete any memory file |
| Fixed file set | No file proliferation — exactly 5 files in v1 |
| Size guidelines | Curated facts, not raw dumps — keeps files useful |
| Time-decay | Old details are compressed, not accumulated forever |
| Self-ingestion guard | Mission sessions excluded from extraction to prevent feedback loops |
| Description = categories | Descriptions list categories of facts, not individual facts — stays stable as content grows |

## Future (v2+)

- More memory files if 5 proves insufficient (e.g., `references.md` for external links)
- `agent_done_decade.md` for multi-year history
- Semantic search (embeddings + SQLite) when memory outgrows what fits in context
- Memory health scoring — detect and auto-recover degraded memories (inspired by OpenClaw)
- Temporal tracking — record how facts change over time (inspired by Zep)
- Export/import — backup memories, share across machines
- Script mode runner (Python) for headless mission extraction with small models
