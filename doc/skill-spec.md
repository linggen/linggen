---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Skills

Skill is all you need.

Linggen is like an OS of agents, skill is the interface. Compared with MCP and tools, skills are extendable and self-explained, so we do skills first in Linggen.

Dynamic extensions: format, discovery, triggers, and tool registration.

Skills are the "dynamic libraries" of Linggen — loaded at runtime, callable by any agent, no code changes needed. Everything that isn't a core built-in tool should be a skill.

Linggen skills follow the [Agent Skills](https://agentskills.io) open standard (spec: https://agentskills.io/specification). Skills are cross-compatible with Claude Code and Codex.

## Related docs

- `session-spec.md`: session-bound skills, effective tools.
- `tool-spec.md`: built-in tools (syscall interface).
- `agent-spec.md`: how agents use skills.
- `product-spec.md`: skills-first design principle.

## Format

Each skill is a directory with `SKILL.md` as entrypoint. The directory name should match the `name` field in frontmatter by convention.

```
my-skill/
├── SKILL.md           # Main instructions (required)
├── references/        # Detailed docs, loaded on demand
│   └── REFERENCE.md
├── scripts/           # Executable code the model can run
│   └── helper.py
└── assets/            # Static resources (templates, schemas)
```

`SKILL.md` has YAML frontmatter + markdown body.

### Progressive disclosure

1. **Metadata** (~100 tokens): `name` + `description` loaded at startup for all skills.
2. **Instructions** (< 5000 tokens recommended): full `SKILL.md` body loaded on activation.
3. **Resources** (as needed): files in `scripts/`, `references/`, `assets/` loaded only when required.

Keep `SKILL.md` under 500 lines. Move detailed reference material to separate files.

### Frontmatter fields

#### Agent Skills standard fields

These fields are defined by the [Agent Skills spec](https://agentskills.io/specification) and work across Linggen, Claude Code, and Codex.

| Field | Required | Purpose |
|:------|:---------|:--------|
| `name` | yes | Lowercase, hyphens only, max 64 chars. Must match directory name. Becomes `/slash-command`. |
| `description` | yes | Max 1024 chars. What the skill does and when to use it. Model reads this to decide. |
| `allowed-tools` | no | Tools permitted when skill is active. |

#### Claude Code extension fields

These fields are defined by Claude Code and also supported by Linggen.

| Field | Purpose |
|:------|:--------|
| `argument-hint` | Autocomplete hint (e.g. `[issue-number]`) |
| `disable-model-invocation` | `true` = only user can invoke |
| `user-invocable` | `false` = only model can invoke |
| `model` | Model preference (`cloud`, `local`, or specific model ID) |
| `context` | `fork` = run in isolated subagent |
| `agent` | Subagent type when `context: fork` |

#### Linggen extension fields

These fields are Linggen-specific extensions.

| Field | Purpose |
|:------|:--------|
| `trigger` | Custom trigger prefix (e.g. `"!!"`, `"%%"`) |
| `app` | App config — makes the skill a directly-runnable app (see below) |
| `permission` | Permission request — user is prompted to approve before skill runs (see below) |
| `install` | Install script — runs once when the skill is installed (see below) |
| `provides` | Capabilities this skill implements — makes it discoverable by Linggen core as a service provider (see below) |
| `implements` | Per-capability bindings — where on the skill's daemon each engine-defined tool is served (see below) |
| `requires` | External dependencies (other skills, binaries, versions) install.sh should resolve |

## Service skills, capabilities, and `implements:`

A skill can advertise that it implements a **capability** — a named service the engine defines and any installable skill can provide. Capabilities turn a skill from "instructions + optional tools" into a **pluggable service implementation**.

The first capability defined is `memory` (see `memory-spec.md`). Others will follow (`search`, `vcs`, `notifications`).

### The split: engine-owned contract, skill-owned backend

Capability tool **names, argument schemas, and permission tiers** live in the engine (`engine::capabilities`). Skills **do not re-declare** these in their manifest — the engine is the single source of truth. A skill just says:

1. Which capabilities it implements (`provides:`).
2. Where on its daemon each tool is served (`implements:`).

This is what makes providers swappable: two different memory skills expose identical tools to the model because both conform to the same engine-defined contract. Only the dispatch target differs.

### Declaring a capability implementation

```yaml
---
name: linggen-memory
description: Semantic memory — facts, activity, semantic retrieval
provides: [memory]
implements:
  memory:
    base_url: http://127.0.0.1:9888
    autostart: "ling-mem start"
    healthcheck: /api/health
    tools:
      Memory_add:    /api/memory/add
      Memory_get:    /api/memory/get
      Memory_search: /api/memory/search
      Memory_list:   /api/memory/list
      Memory_update: /api/memory/update
      Memory_delete: /api/memory/delete
      Memory_forget: /api/memory/forget
install: install.sh
---
```

`provides:` is a list (a skill can implement more than one capability). `implements:` is a map keyed by capability name; one entry per capability claimed. The engine scans all installed skills on load and builds a `capability → active-skill` map plus a per-skill binding table.

### `implements.<capability>` fields

| Field | Purpose |
|:------|:--------|
| `base_url` | Root of the skill's HTTP surface. Include scheme + host + port. The engine concatenates this with each tool's path to form the dispatch URL. |
| `autostart` | Command the engine runs when the daemon isn't reachable on the first call. Whitespace-split. First token resolved as `$SKILL_DIR/bin/<name>` first, else bare name on `$PATH`. Default: `ling-mem start`. |
| `healthcheck` | Path returning 200 when the daemon is healthy. Default: `/api/health`. Reserved for the engine's future liveness probe. |
| `tools` | Map from capability tool name (engine-defined) → path on the daemon. Must cover every tool in the capability's contract. |

### How dispatch works

When the model calls a capability tool (e.g. `Memory_search`):

```
Model calls Memory_search({query: "dock calibration"})
  → Engine capability registry:  Memory_search ∈ capability "memory"
  → Active provider for memory:  linggen-memory
  → Read implements.memory.tools[Memory_search]:  /api/memory/search
  → URL = base_url + path:       http://127.0.0.1:9888/api/memory/search
  → POST JSON args, parse {ok, data} envelope
  → Return data to the model
```

On connection refuse / timeout, the engine spawns `implements.memory.autostart` and retries once. The daemon outlives the Linggen process — the engine never auto-stops it.

If no skill is registered for a capability, the tool is filtered out of the model's tool list entirely — the model doesn't see it.

Resolution is deterministic when multiple skills claim the same capability: Project-sourced skills beat Compat, which beat Global; ties break by name. Only one provider per capability is active at a time in v1; multi-provider merging is deferred.

### Swapping providers

Because tool names / schemas / tiers are engine-owned and capability-namespaced (`Memory_*` instead of provider-specific), users can swap providers without the model seeing any change. A new provider implements the same contract; the model continues to call `Memory_search` with the same arg shape.

### When to use `provides:` + `implements:` vs plain `tools:`

- **Use `provides:` + `implements:`** when the skill implements a Linggen-defined capability contract (memory, search, …). Tool names, schemas, and tiers are engine-owned; this skill is one of possibly many interchangeable backends.
- **Use plain `tools:` registration** when the skill ships a tool unique to itself (e.g. `run_lint` on a linter skill, `DashboardUpdate` on an app skill). The skill owns the name, schema, and tier; nothing else can implement it.

Capability dispatch is strictly for skills that fit a well-known service slot. A skill can declare both — canonical contract coverage via `implements:` plus its own unique tools via `tools:`.

## Skill install

Skills can declare an `install` field pointing to a script that runs once on installation. The script handles any setup the skill needs — creating directories, copying templates, registering missions.

```yaml
install: scripts/install.sh
```

The script runs with `$SKILL_DIR` set to the skill's directory. It should be **idempotent** — safe to run multiple times (skip files that already exist).

### What install scripts do

- Create directories (e.g. `~/.linggen/memory/`)
- Copy template files from the skill's `assets/` directory
- Copy mission files to `~/.linggen/missions/{name}/` (replaces the old `mission:` frontmatter field)
- Any other one-time setup

### When install scripts run

| Entry point | Trigger |
|:-----------|:--------|
| `ling init` | Runs install scripts for all installed skills |
| WebUI "Install" button | Runs after skill files are copied |
| `ling skills install` | Runs after download and extraction |
| Auto-install on first startup | Runs after built-in skills are downloaded |

All paths converge on the same `run_install_script()` function.

### Example: memory skill

```yaml
name: memory
install: scripts/install.sh
```

```bash
#!/usr/bin/env bash
# install.sh — Bootstrap memory files and mission
MEMORY_DIR="$HOME/.linggen/memory"
mkdir -p "$MEMORY_DIR"
for f in "$SKILL_DIR/assets/"*.md; do
  target="$MEMORY_DIR/$(basename "$f")"
  [ -f "$target" ] || cp "$f" "$target"
done

MISSION_DIR="$HOME/.linggen/missions/memory"
if [ ! -d "$MISSION_DIR" ]; then
  mkdir -p "$MISSION_DIR"
  cp "$SKILL_DIR/assets/mission.md" "$MISSION_DIR/mission.md"
fi
```

### Mission as an asset

Skills that need a cron mission ship a `mission.md` file in their `assets/` directory. The install script copies it to `~/.linggen/missions/{name}/`. The mission scheduler picks it up automatically (missions are cached in memory and reloaded after skill install).

This replaces the old `mission:` frontmatter field — the mission definition is a file, not a config property.

## Skill permissions

Skills can declare a `permission` field to request elevated access. When a skill with a permission request is invoked, the user is prompted to approve before execution. See `permission-spec.md` for the full permission model.

```yaml
permission:
  mode: admin          # "read", "edit", or "admin"
  paths: ["/", "~"]    # Paths to grant the mode on
  warning: "This skill runs system commands that modify files"
```

| Field | Required | Description |
|:------|:---------|:------------|
| `mode` | yes | Required permission mode: `read`, `edit`, or `admin` |
| `paths` | no | Paths to grant the mode on (default: workspace root) |
| `warning` | no | Warning message shown to user before approval |

If a skill only reads data (e.g. search, status checks), it should use `mode: read`. Skills that write files should use `mode: edit`. Skills that run arbitrary shell commands should use `mode: admin`.

## App skills

Skills can act as **apps** — directly executable with a custom UI. When a skill has an `app` section in frontmatter, invoking it opens the UI.

### Two kinds of app skills

| Kind | `allowed-tools` | Model involvement | Example |
|:-----|:-----------------|:------------------|:--------|
| **Standalone** | n/a | None — pure frontend | `arcade-game` (Snake, Pong, Tetris) |
| **Interactive** | any (often `[]`) | App UI talks to ling via session API | `game-table` (chess, poker vs AI) |

Standalone apps bypass the model entirely. Interactive apps create a **session-bound skill** — the app UI creates a session with `skill: "game-table"`, and every message in that session activates the skill (tool restrictions, skill body injection).

### App fields

| Field | Required | Description |
|:------|:---------|:------------|
| `launcher` | yes | `web`, `bash`, or `url` |
| `entry` | yes | Filename (web/bash) or URL (url launcher) |
| `width` | no | Suggested panel width in pixels |
| `height` | no | Suggested panel height in pixels |

### Launcher types

| Launcher | Behavior |
|:---------|:---------|
| `web` | Serve skill directory as static files, open in embedded panel (iframe) |
| `bash` | Execute `entry` script, stream output |
| `url` | Open external URL in browser or panel |

### Interactive app pattern

Interactive app skills use the existing linggen API from within the iframe (same-origin):

1. `GET /api/models` — populate model picker
2. `POST /api/sessions` with `skill` field — create skill-bound session
3. `POST /api/run` — send messages to ling (shaped by skill)
4. Events streamed via WebRTC data channel for the session

The skill's `allowed-tools` restricts ling's tools for the session. The skill body is injected into every system prompt. No custom endpoints needed.

### Static file serving

Web apps are served at `/apps/{skill-name}/`. Scoped to skill directory (no path traversal).

## Skill activation modes

| Mode | Trigger | Scope | Tool restriction |
|:-----|:--------|:------|:-----------------|
| **Transient** | `/skill-name` in chat | Single invocation | During that run, then restored |
| **Session-bound** | Session created with `skill` field | Entire session | Every message in session |

App skills with interactive UIs are session-bound. The iframe creates the session with the skill binding. All messages in that session get the skill's tool restrictions and instructions.

Session-bound skills combine with the agent's system prompt: `effective_tools = intersection(agent.tools, skill.allowed-tools)`. When effective tools is empty, tool-related prompt sections (schemas, usage guidelines, delegation, plan mode) are skipped entirely.

## Discovery

Skills are discovered at startup and on file change (live reload).

**Discovery paths** (later overrides earlier):

| Priority | Path | Scope |
|:---------|:-----|:------|
| 1 | `~/.linggen/skills/<name>/SKILL.md` | Global personal |
| 2 | `~/.claude/skills/`, `~/.codex/skills/` | Cross-tool compat |
| 3 | `.linggen/skills/<name>/SKILL.md` | Project (highest priority) |

All skill metadata (name + description + full body) is loaded at startup. Descriptions are included in agent context so the model knows what's available.

## Invocation

Two ways to invoke a skill:

1. **User**: type `/skill-name [args]` in chat.
2. **Model**: model decides to invoke based on description match.

Control who can invoke:
- Default: both user and model.
- `disable-model-invocation: true`: user only.
- `user-invocable: false`: model only.

## Trigger symbols

Parsed from user input only (model output is not parsed):

- `/` — built-in commands + skill invocation.
- `@` — file mentions (aligned with Claude Code).
- Custom triggers declared in frontmatter.

**Matching order**: system triggers → user-defined triggers → pass-through to model.

## Skill tools

Skills can define tool functions via `tools` in their frontmatter. These register dynamically in the tool registry alongside built-in tools. Available only when the skill is active (session-bound or transient invocation).

### Command tools

Execute a shell command with template substitution:

```yaml
tools:
  - name: run_lint
    description: Run project linter
    cmd: "cd $SKILL_DIR && ./scripts/lint.sh {{path}}"
    args:
      path: { type: string, required: true, description: "File to lint" }
```

- Executed via `sh -c` with `{{param}}` substitution.
- `$SKILL_DIR` resolves to the skill's directory path.
- Same timeout and safety validation as the Bash tool.

### Data tools

Pass structured data from the agent to the skill's UI — no shell command, no side effects. Defined by omitting `cmd`:

```yaml
tools:
  - name: DashboardUpdate
    description: Send scan results to the dashboard UI
    args:
      system: { type: object, description: "System info" }
      disk: { type: object, description: "Disk usage" }
```

When the agent calls a data tool:
1. The engine emits a `content_block` event with the tool name and args.
2. The skill app receives it via `onContentBlock` callback (see App skills).
3. The tool returns `"ok"` — the value is in the event, not the return.

Data tools enable **real-time structured updates** from agent to app UI without text-tag parsing hacks. The agent can call them multiple times for incremental updates.

**Implementation**: `engine/skill_tool.rs`, `engine/tool_registry.rs`

### Built-in `PageUpdate` (app skills)

Every skill with an `app` section automatically receives a built-in data tool called `PageUpdate`. Skills do **not** need to declare it in their `tools:` list.

```
PageUpdate({ "page": { <skill-specific layout> } })
```

- Emitted as a `content_block` event; the app iframe receives it via `onContentBlock` and re-renders.
- The `page` argument is opaque to the engine — each skill defines its own layout schema in SKILL.md (e.g. `top_bar`, `body`, `footer` for dashboard-style skills).
- When the active skill has an `app`, the system prompt includes a standing instruction to call `PageUpdate` whenever state the user should see changes. Skills should **not** emit page JSON as text — always use the tool.

This replaces the older `<!--page-->` text-tag convention, which required per-skill parsing and nag loops. Apps can still parse text tags for backward compatibility, but new skills should use `PageUpdate`.

## Cross-tool compatibility

Linggen follows the [Agent Skills](https://agentskills.io) open standard. Skills written for Linggen work in Claude Code and Codex — same directory structure, same frontmatter. Claude Code extension fields (`argument-hint`, `disable-model-invocation`, `context`, etc.) are also supported. Linggen-specific fields (`trigger`, `app`) are ignored by other tools.

Skills installed in `~/.claude/skills/` or `.claude/skills/` are discovered by Linggen, Claude Code, and Codex automatically.
