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

## Cross-tool compatibility

Linggen follows the [Agent Skills](https://agentskills.io) open standard. Skills written for Linggen work in Claude Code and Codex — same directory structure, same frontmatter. Claude Code extension fields (`argument-hint`, `disable-model-invocation`, `context`, etc.) are also supported. Linggen-specific fields (`trigger`, `app`) are ignored by other tools.

Skills installed in `~/.claude/skills/` or `.claude/skills/` are discovered by Linggen, Claude Code, and Codex automatically.
