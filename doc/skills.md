---
type: spec
guide: |
  Product specification — describe what the system should do and why.
  Keep it brief. Aim to guide design and implementation, not document code.
  Avoid implementation details like function signatures, variable types, or code snippets.
---

# Skills

Skill is all you need.

Linggen is like a OS of agent, skill is the interface. Comparing with mcp, tools, skill is extendable, self-explained, so we do skill first in linggen. 

Dynamic extensions: format, discovery, triggers, and tool registration.

Skills are the "dynamic libraries" of linggen — loaded at runtime, callable by any agent, no code changes needed. Everything that isn't a core built-in tool should be a skill.



## Related docs

- `tools.md`: built-in tools (syscall interface).
- `agents.md`: how agents use skills.
- `product-spec.md`: skills-first design principle.

## Format

Each skill is a directory with `SKILL.md` as entrypoint:

```
my-skill/
├── SKILL.md           # Main instructions (required)
├── template.md        # Template for model to fill in
├── examples/          # Example outputs
└── scripts/
    └── helper.py      # Script the model can execute
```

`SKILL.md` has YAML frontmatter + markdown instructions.

### Frontmatter fields

| Field | Purpose |
|:------|:--------|
| `name` | Display name, becomes `/slash-command` |
| `description` | When to use (model reads this to decide) |
| `argument-hint` | Autocomplete hint (e.g. `[issue-number]`) |
| `disable-model-invocation` | `true` = only user can invoke |
| `user-invocable` | `false` = only model can invoke |
| `allowed-tools` | Tools permitted when skill is active |
| `model` | Model preference (`cloud`, `local`, or specific model ID) |
| `context` | `fork` = run in isolated subagent |
| `agent` | Subagent type when `context: fork` |
| `trigger` | Custom trigger prefix (e.g. `"!!"`, `"%%"`) |

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

Skills written for Linggen work in Claude Code and Codex — same Agent Skills standard, same directory structure, same frontmatter.

The `linggen` skill (in `~/.claude/skills/linggen/`) lets other AI tools dispatch tasks to Linggen.
