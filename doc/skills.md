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

- `tools.md`: built-in tools (syscall interface).
- `agents.md`: how agents use skills.
- `product-spec.md`: skills-first design principle.

## Format

Each skill is a directory with `SKILL.md` as entrypoint. The directory name **must match** the `name` field in frontmatter.

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
| `license` | no | License name or reference to bundled file. |
| `compatibility` | no | Environment requirements (products, packages, network). |
| `metadata` | no | Arbitrary key-value pairs for extra info (author, version). |
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

## App skills

Skills can act as **apps** — directly executable without model involvement. When a skill has an `app` section in frontmatter, invoking it (via `/skill-name` or clicking "Launch" in the UI) runs it immediately, bypassing the model entirely.

```yaml
---
name: arcade-game
description: Retro arcade games
app:
  launcher: web
  entry: scripts/index.html
  width: 800
  height: 600
---
```

### Launcher types

| Launcher | Behavior |
|:---------|:---------|
| `web` | Serve skill directory as static files, open in embedded panel (iframe) |
| `bash` | Execute `entry` script, stream output |
| `url` | Open external URL in browser or panel |

### App fields

| Field | Required | Description |
|:------|:---------|:------------|
| `launcher` | yes | `web`, `bash`, or `url` |
| `entry` | yes | Filename (web/bash) or URL (url launcher) |
| `width` | no | Suggested panel width in pixels |
| `height` | no | Suggested panel height in pixels |

### Invocation

- **User**: `/skill-name` or click "Launch" button on the skill card in the Web UI.
- **Model**: call the `RunApp` built-in tool (see `tools.md`).

Both paths skip the model for the launch itself. The model can call `RunApp` during a conversation to open an app for the user.

### Static file serving

Web apps are served at `/apps/{skill-name}/`. The server resolves the skill's directory and serves files statically, scoped to the skill directory (no path traversal).

## Discovery

Skills are discovered at startup and on file change (live reload).

**Discovery paths** (higher priority wins):

| Level | Path | Scope |
|:------|:-----|:------|
| Personal | `~/.linggen/skills/<name>/SKILL.md` | All projects |
| Project | `.linggen/skills/<name>/SKILL.md` | This project only |
| Compat | `~/.claude/skills/`, `~/.codex/skills/` | Cross-tool compatibility |

Descriptions are loaded into agent context so the model knows what's available. Full content loads only when invoked.

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
- `@` — mentions, routed to skills for the named target.
- Custom triggers declared in frontmatter.

**Matching order**: system triggers → user-defined triggers → pass-through to model.

## Skill tools

Skills can define tool functions via `tool_defs` in their metadata. These register dynamically in the tool registry alongside built-in tools.

- Skill tools execute as subprocesses (`sh -c`) with template substitution (`{{param}}`).
- Schemas are generated dynamically from skill definitions.
- Same command validation as Bash tool.

**Implementation**: `engine/skill_tool.rs`, `engine/tool_registry.rs`

## Cross-tool compatibility

Linggen follows the [Agent Skills](https://agentskills.io) open standard. Skills written for Linggen work in Claude Code and Codex — same directory structure, same frontmatter. Claude Code extension fields (`argument-hint`, `disable-model-invocation`, `context`, etc.) are also supported. Linggen-specific fields (`trigger`, `app`) are ignored by other tools.

Claude Code now treats `.claude/commands/` and `.claude/skills/` as equivalent — a command at `.claude/commands/deploy.md` and a skill at `.claude/skills/deploy/SKILL.md` both create `/deploy`. Linggen supports both via compat discovery paths.

Skills installed in `~/.claude/skills/` or `.claude/skills/` are discovered by Linggen, Claude Code, and Codex automatically.
